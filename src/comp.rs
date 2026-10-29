//! Compositeur Wayland embarqué (Smithay).
//!
//! Il tourne dans son propre thread, sans backend d'affichage : les clients dessinent
//! via wl_shm, on copie leurs pixels et on les envoie au monde 3D (thread Bevy).
//! Les entrées (souris/clavier) font le chemin inverse.

use std::{
    collections::HashMap,
    sync::{
        mpsc::{channel, Receiver, Sender, TryRecvError},
        Arc,
    },
    time::{Duration, Instant},
};

use smithay::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm,
    delegate_xdg_shell,
    input::{
        keyboard::{FilterResult, KeyboardHandle},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerHandle},
        Seat, SeatHandler, SeatState,
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_buffer, wl_seat, wl_surface::WlSurface},
            Client, Display, ListeningSocket,
        },
    },
    utils::{Serial, Transform, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            with_states, with_surface_tree_downward, BufferAssignment, CompositorClientState,
            CompositorHandler, CompositorState, SurfaceAttributes, TraversalAction,
        },
        output::OutputHandler,
        selection::{
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
        shm::{with_buffer_contents, BufferData, ShmHandler, ShmState},
    },
};
use smithay::reexports::wayland_server::protocol::wl_shm;
use std::os::unix::io::OwnedFd;

/// Identifiant stable d'une fenêtre (app) côté monde 3D.
pub type WinId = u32;

/// Compositeur -> monde 3D.
pub enum ToWorld {
    Created { id: WinId },
    Meta { id: WinId, title: String, app_id: String },
    Frame { id: WinId, width: u32, height: u32, rgba: Vec<u8> },
    Closed { id: WinId },
}

/// Monde 3D -> compositeur.
pub enum ToComp {
    /// Le pointeur est sur la fenêtre `id` à la position (x, y) en pixels de la fenêtre.
    PointerMove { id: WinId, x: f64, y: f64 },
    PointerLeave,
    Button { code: u32, pressed: bool },
    Scroll { dx: f64, dy: f64 },
    /// `code` = code evdev (le compositeur ajoute 8 pour xkb).
    Key { code: u32, pressed: bool },
    /// Donne (ou retire) le focus clavier.
    Focus(Option<WinId>),
    Resize { id: WinId, width: u32, height: u32 },
    Close { id: WinId },
}

struct Win {
    id: WinId,
    toplevel: ToplevelSurface,
    announced: bool,
}

struct App {
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    seat_state: SeatState<Self>,
    data_device_state: DataDeviceState,
    seat: Seat<Self>,

    wins: Vec<Win>,
    next_id: WinId,
    to_world: Sender<ToWorld>,
    default_size: (i32, i32),
}

impl App {
    fn win_of_surface(&mut self, surface: &WlSurface) -> Option<&mut Win> {
        self.wins.iter_mut().find(|w| w.toplevel.wl_surface() == surface)
    }
    fn surface_of(&self, id: WinId) -> Option<WlSurface> {
        self.wins.iter().find(|w| w.id == id).map(|w| w.toplevel.wl_surface().clone())
    }
}

impl BufferHandler for App {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl XdgShellHandler for App {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let id = self.next_id;
        self.next_id += 1;
        let (w, h) = self.default_size;
        surface.with_pending_state(|s| {
            s.size = Some((w, h).into());
            s.states.set(xdg_toplevel::State::Activated);
        });
        surface.send_configure();
        self.wins.push(Win { id, toplevel: surface, announced: false });
        let _ = self.to_world.send(ToWorld::Created { id });
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(pos) = self.wins.iter().position(|w| w.toplevel == surface) {
            let w = self.wins.remove(pos);
            let _ = self.to_world.send(ToWorld::Closed { id: w.id });
        }
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}
    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}
    fn reposition_request(&mut self, _s: PopupSurface, _p: PositionerState, _token: u32) {}
}

impl SelectionHandler for App {
    type SelectionUserData = ();
}
impl DataDeviceHandler for App {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}
impl ClientDndGrabHandler for App {}
impl ServerDndGrabHandler for App {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

impl CompositorHandler for App {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        // On ne s'intéresse qu'aux surfaces racines (toplevels). Sous-surfaces/popups : plus tard.
        let Some(win) = self.win_of_surface(surface) else { return };
        let id = win.id;
        let first = !win.announced;
        win.announced = true;

        if first {
            let (title, app_id) = with_states(surface, |states| {
                let data = states.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap();
                (data.title.clone().unwrap_or_default(), data.app_id.clone().unwrap_or_default())
            });
            let _ = self.to_world.send(ToWorld::Meta { id, title, app_id });
        }

        // Récupère le buffer attaché (et le libère dès qu'on a copié les pixels).
        let assignment = with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            guard.current().buffer.take()
        });
        if let Some(BufferAssignment::NewBuffer(buffer)) = assignment {
            let copied = with_buffer_contents(&buffer, |ptr, len, data| copy_shm(ptr, len, data));
            if let Ok(Some((w, h, rgba))) = copied {
                let _ = self.to_world.send(ToWorld::Frame { id, width: w, height: h, rgba });
            }
            buffer.release();
        }
    }
}

/// Convertit un buffer wl_shm (ARGB/XRGB 8888, little-endian = BGRA en mémoire) en RGBA.
fn copy_shm(ptr: *const u8, len: usize, data: BufferData) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = (data.width as usize, data.height as usize);
    let stride = data.stride as usize;
    let off = data.offset as usize;
    if w == 0 || h == 0 || off + stride * (h - 1) + w * 4 > len {
        return None;
    }
    let opaque = matches!(data.format, wl_shm::Format::Xrgb8888);
    if !matches!(data.format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888) {
        return None;
    }
    let src = unsafe { std::slice::from_raw_parts(ptr, len) };
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let row = &src[off + y * stride..off + y * stride + w * 4];
        let dst = &mut out[y * w * 4..(y + 1) * w * 4];
        for x in 0..w {
            let (b, g, r, a) = (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]);
            dst[x * 4] = r;
            dst[x * 4 + 1] = g;
            dst[x * 4 + 2] = b;
            dst[x * 4 + 3] = if opaque { 255 } else { a };
        }
    }
    Some((w as u32, h as u32, out))
}

impl ShmHandler for App {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for App {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}
    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: smithay::input::pointer::CursorImageStatus) {}
}

impl OutputHandler for App {}

#[derive(Default)]
struct ClientState {
    compositor_state: CompositorClientState,
}
impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

delegate_xdg_shell!(App);
delegate_compositor!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_data_device!(App);
delegate_output!(App);

fn send_frames(surface: &WlSurface, time: u32) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |_s, states, _| {
            for cb in states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .frame_callbacks
                .drain(..)
            {
                cb.done(time);
            }
        },
        |_, _, _| true,
    );
}

/// Handle vers le compositeur lancé dans son thread.
pub struct CompHandle {
    pub socket: String,
    pub rx: Receiver<ToWorld>,
    pub tx: Sender<ToComp>,
}

/// Lance le compositeur sur le socket Wayland `socket` (ex: "wayworld-0").
pub fn spawn(socket: &str, default_size: (i32, i32)) -> std::io::Result<CompHandle> {
    let (to_world, rx) = channel::<ToWorld>();
    let (tx, from_world) = channel::<ToComp>();
    let listener = ListeningSocket::bind(socket)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let name = socket.to_string();

    std::thread::Builder::new().name("wayworld-comp".into()).spawn(move || {
        if let Err(e) = run(listener, to_world, from_world, default_size) {
            eprintln!("[comp] erreur fatale : {e}");
        }
    })?;
    Ok(CompHandle { socket: name, rx, tx })
}

fn run(
    listener: ListeningSocket,
    to_world: Sender<ToWorld>,
    from_world: Receiver<ToComp>,
    default_size: (i32, i32),
) -> Result<(), Box<dyn std::error::Error>> {
    let mut display: Display<App> = Display::new()?;
    let dh = display.handle();

    let mut seat_state = SeatState::new();
    let seat = seat_state.new_wl_seat(&dh, "wayworld");
    let mut state = App {
        compositor_state: CompositorState::new::<App>(&dh),
        xdg_shell_state: XdgShellState::new::<App>(&dh),
        shm_state: ShmState::new::<App>(&dh, vec![]),
        seat_state,
        data_device_state: DataDeviceState::new::<App>(&dh),
        seat,
        wins: Vec::new(),
        next_id: 1,
        to_world,
        default_size,
    };

    // Sortie virtuelle (beaucoup d'apps en exigent une).
    let output = Output::new(
        "WORLD-1".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "wayworld".into(),
            model: "virtual".into(),
        },
    );
    let _global = output.create_global::<App>(&dh);
    let mode = Mode { size: (1920, 1080).into(), refresh: 60_000 };
    output.change_current_state(Some(mode), Some(Transform::Normal), None, Some((0, 0).into()));
    output.set_preferred(mode);

    let keyboard: KeyboardHandle<App> = state.seat.add_keyboard(Default::default(), 300, 30)?;
    let pointer: PointerHandle<App> = state.seat.add_pointer();

    let start = Instant::now();
    let mut last_frame = Instant::now();
    let mut pointer_win: Option<WinId> = None;
    let mut clients = Vec::new();

    loop {
        // Nouveaux clients
        while let Some(stream) = listener.accept()? {
            let client = display.handle().insert_client(stream, Arc::new(ClientState::default()))?;
            clients.push(client);
        }

        // Entrées venant du monde 3D
        loop {
            let msg = match from_world.try_recv() {
                Ok(m) => m,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            };
            let now = start.elapsed().as_millis() as u32;
            match msg {
                ToComp::PointerMove { id, x, y } => {
                    if let Some(surface) = state.surface_of(id) {
                        pointer_win = Some(id);
                        pointer.motion(
                            &mut state,
                            Some((surface, (0.0, 0.0).into())),
                            &MotionEvent { location: (x, y).into(), serial: SERIAL_COUNTER.next_serial(), time: now },
                        );
                        pointer.frame(&mut state);
                    }
                }
                ToComp::PointerLeave => {
                    if pointer_win.take().is_some() {
                        pointer.motion(
                            &mut state,
                            None,
                            &MotionEvent { location: (0.0, 0.0).into(), serial: SERIAL_COUNTER.next_serial(), time: now },
                        );
                        pointer.frame(&mut state);
                    }
                }
                ToComp::Button { code, pressed } => {
                    pointer.button(
                        &mut state,
                        &ButtonEvent {
                            button: code,
                            state: if pressed {
                                smithay::backend::input::ButtonState::Pressed
                            } else {
                                smithay::backend::input::ButtonState::Released
                            },
                            serial: SERIAL_COUNTER.next_serial(),
                            time: now,
                        },
                    );
                    pointer.frame(&mut state);
                }
                ToComp::Scroll { dx, dy } => {
                    let mut frame = AxisFrame::new(now);
                    if dx != 0.0 {
                        frame = frame.value(smithay::backend::input::Axis::Horizontal, dx);
                    }
                    if dy != 0.0 {
                        frame = frame.value(smithay::backend::input::Axis::Vertical, dy);
                    }
                    pointer.axis(&mut state, frame);
                    pointer.frame(&mut state);
                }
                ToComp::Key { code, pressed } => {
                    keyboard.input::<(), _>(
                        &mut state,
                        (code + 8).into(),
                        if pressed {
                            smithay::backend::input::KeyState::Pressed
                        } else {
                            smithay::backend::input::KeyState::Released
                        },
                        SERIAL_COUNTER.next_serial(),
                        now,
                        |_, _, _| FilterResult::Forward,
                    );
                }
                ToComp::Focus(target) => {
                    let surface = target.and_then(|id| state.surface_of(id));
                    keyboard.set_focus(&mut state, surface, SERIAL_COUNTER.next_serial());
                }
                ToComp::Resize { id, width, height } => {
                    if let Some(w) = state.wins.iter().find(|w| w.id == id) {
                        w.toplevel.with_pending_state(|s| s.size = Some((width as i32, height as i32).into()));
                        w.toplevel.send_pending_configure();
                    }
                }
                ToComp::Close { id } => {
                    if let Some(w) = state.wins.iter().find(|w| w.id == id) {
                        w.toplevel.send_close();
                    }
                }
            }
        }

        // Frame callbacks ~60 Hz
        if last_frame.elapsed() >= Duration::from_millis(16) {
            last_frame = Instant::now();
            let t = start.elapsed().as_millis() as u32;
            for w in &state.wins {
                send_frames(w.toplevel.wl_surface(), t);
            }
        }

        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;
        std::thread::sleep(Duration::from_millis(2));
    }
}