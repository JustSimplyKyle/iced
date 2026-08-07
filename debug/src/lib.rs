pub use iced_core as core;
pub use iced_futures as futures;

use crate::core::theme;
use crate::core::window;
use crate::futures::Subscription;

pub use internal::Span;

#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    pub name: &'static str,
    pub theme: Option<theme::Palette>,
    pub can_time_travel: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Primitive {
    Quad,
    Triangle,
    Shader,
    Image,
    Text,
}

#[derive(Debug, Clone, Copy)]
pub enum Command {
    RewindTo { message: usize },
    GoLive,
}

pub fn enable() {
    internal::enable();
}

pub fn disable() {
    internal::disable();
}

pub fn init(metadata: Metadata) {
    internal::init(metadata);
    hot::init();
}

pub fn quit() -> bool {
    internal::quit()
}

pub fn theme_changed(f: impl FnOnce() -> Option<theme::Palette>) {
    internal::theme_changed(f);
}

pub fn tasks_spawned(amount: usize) {
    internal::tasks_spawned(amount);
}

pub fn subscriptions_tracked(amount: usize) {
    internal::subscriptions_tracked(amount);
}

pub fn layers_rendered(amount: impl FnOnce() -> usize) {
    internal::layers_rendered(amount);
}

pub fn boot() -> Span {
    internal::boot()
}

pub fn update(message: &impl std::fmt::Debug) -> Span {
    internal::update(message)
}

pub fn view(window: window::Id) -> Span {
    internal::view(window)
}

pub fn layout(window: window::Id) -> Span {
    internal::layout(window)
}

pub fn interact(window: window::Id) -> Span {
    internal::interact(window)
}

pub fn draw(window: window::Id) -> Span {
    internal::draw(window)
}

pub fn prepare(primitive: Primitive) -> Span {
    internal::prepare(primitive)
}

pub fn render(primitive: Primitive) -> Span {
    internal::render(primitive)
}

pub fn present(window: window::Id) -> Span {
    internal::present(window)
}

pub fn time(name: impl Into<String>) -> Span {
    internal::time(name)
}

pub fn time_with<T>(name: impl Into<String>, f: impl FnOnce() -> T) -> T {
    let span = time(name);
    let result = f();
    span.finish();

    result
}

pub fn commands() -> Subscription<Command> {
    internal::commands()
}

pub fn hot<O>(f: impl FnOnce() -> O) -> O {
    hot::call(f)
}

pub fn on_hotpatch(f: impl Fn() + Send + Sync + 'static) {
    hot::on_hotpatch(f)
}

pub fn is_stale() -> bool {
    hot::is_stale()
}

#[cfg(all(feature = "enable", not(target_arch = "wasm32")))]
mod internal {
    use crate::core::theme;
    use crate::core::time::Instant;
    use crate::core::window;
    use crate::futures::Subscription;
    use crate::futures::futures::Stream;
    use crate::{Command, Metadata, Primitive};

    use iced_beacon as beacon;

    use beacon::client::{self, Client};
    use beacon::span;
    use beacon::span::present;

    use std::sync::atomic::{self, AtomicBool, AtomicUsize};
    use std::sync::{LazyLock, RwLock};

    pub fn init(metadata: Metadata) {
        let name = metadata.name.split("::").next().unwrap_or(metadata.name);

        *METADATA.write().expect("Write application metadata") =
            client::Metadata {
                name,
                theme: metadata.theme,
                can_time_travel: metadata.can_time_travel,
            };
    }

    pub fn quit() -> bool {
        if BEACON.is_connected() {
            BEACON.quit();

            true
        } else {
            false
        }
    }

    pub fn theme_changed(f: impl FnOnce() -> Option<theme::Palette>) {
        let Some(palette) = f() else {
            return;
        };

        if METADATA.read().expect("Read last palette").theme.as_ref()
            != Some(&palette)
        {
            log(client::Event::ThemeChanged(palette));

            METADATA.write().expect("Write last palette").theme = Some(palette);
        }
    }

    pub fn tasks_spawned(amount: usize) {
        log(client::Event::CommandsSpawned(amount));
    }

    pub fn subscriptions_tracked(amount: usize) {
        log(client::Event::SubscriptionsTracked(amount));
    }

    pub fn layers_rendered(amount: impl FnOnce() -> usize) {
        log(client::Event::LayersRendered(amount()));
    }

    pub fn boot() -> Span {
        span(span::Stage::Boot)
    }

    pub fn update(message: &impl std::fmt::Debug) -> Span {
        let span = span(span::Stage::Update);

        let number = LAST_UPDATE.fetch_add(1, atomic::Ordering::Relaxed);

        let start = Instant::now();
        let message = format!("{message:?}");
        let elapsed = start.elapsed();

        if elapsed.as_millis() >= 1 {
            log::warn!(
                "Slow `Debug` implementation of `Message` (took {elapsed:?})!"
            );
        }

        let message = if message.len() > 49 {
            message
                .chars()
                .take(49)
                .chain("...".chars())
                .collect::<String>()
        } else {
            message
        };

        log(client::Event::MessageLogged { number, message });

        span
    }

    pub fn view(window: window::Id) -> Span {
        span(span::Stage::View(window))
    }

    pub fn layout(window: window::Id) -> Span {
        span(span::Stage::Layout(window))
    }

    pub fn interact(window: window::Id) -> Span {
        span(span::Stage::Interact(window))
    }

    pub fn draw(window: window::Id) -> Span {
        span(span::Stage::Draw(window))
    }

    pub fn prepare(primitive: Primitive) -> Span {
        span(span::Stage::Prepare(to_primitive(primitive)))
    }

    pub fn render(primitive: Primitive) -> Span {
        span(span::Stage::Render(to_primitive(primitive)))
    }

    pub fn present(window: window::Id) -> Span {
        span(span::Stage::Present(window))
    }

    pub fn time(name: impl Into<String>) -> Span {
        span(span::Stage::Custom(name.into()))
    }

    pub fn enable() {
        ENABLED.store(true, atomic::Ordering::Relaxed);
    }

    pub fn disable() {
        ENABLED.store(false, atomic::Ordering::Relaxed);
    }

    pub fn commands() -> Subscription<Command> {
        fn listen_for_commands() -> impl Stream<Item = Command> {
            use crate::futures::futures::stream;

            stream::unfold(BEACON.subscribe(), async move |mut receiver| {
                let command = match receiver.recv().await? {
                    client::Command::RewindTo { message } => {
                        Command::RewindTo { message }
                    }
                    client::Command::GoLive => Command::GoLive,
                };

                Some((command, receiver))
            })
        }

        Subscription::run(listen_for_commands)
    }

    fn span(span: span::Stage) -> Span {
        log(client::Event::SpanStarted(span.clone()));

        Span {
            span,
            start: Instant::now(),
        }
    }

    fn to_primitive(primitive: Primitive) -> present::Primitive {
        match primitive {
            Primitive::Quad => present::Primitive::Quad,
            Primitive::Triangle => present::Primitive::Triangle,
            Primitive::Shader => present::Primitive::Shader,
            Primitive::Text => present::Primitive::Text,
            Primitive::Image => present::Primitive::Image,
        }
    }

    fn log(event: client::Event) {
        if ENABLED.load(atomic::Ordering::Relaxed) {
            BEACON.log(event);
        }
    }

    #[derive(Debug)]
    pub struct Span {
        span: span::Stage,
        start: Instant,
    }

    impl Span {
        pub fn finish(self) {
            log(client::Event::SpanFinished(self.span, self.start.elapsed()));
        }
    }

    static BEACON: LazyLock<Client> = LazyLock::new(|| {
        let metadata = METADATA.read().expect("Read application metadata");

        client::connect(metadata.clone())
    });

    static METADATA: RwLock<client::Metadata> = RwLock::new(client::Metadata {
        name: "",
        theme: None,
        can_time_travel: false,
    });

    static LAST_UPDATE: AtomicUsize = AtomicUsize::new(0);
    static ENABLED: AtomicBool = AtomicBool::new(true);
}

#[cfg(any(not(feature = "enable"), target_arch = "wasm32"))]
mod internal {
    use crate::core::theme;
    use crate::core::window;
    use crate::futures::Subscription;
    use crate::{Command, Metadata, Primitive};

    pub fn enable() {}
    pub fn disable() {}

    pub fn init(_metadata: Metadata) {}

    pub fn quit() -> bool {
        false
    }

    pub fn theme_changed(_f: impl FnOnce() -> Option<theme::Palette>) {}

    pub fn tasks_spawned(_amount: usize) {}

    pub fn subscriptions_tracked(_amount: usize) {}

    pub fn layers_rendered(_amount: impl FnOnce() -> usize) {}

    pub fn boot() -> Span {
        Span
    }

    pub fn update(_message: &impl std::fmt::Debug) -> Span {
        Span
    }

    pub fn view(_window: window::Id) -> Span {
        Span
    }

    pub fn layout(_window: window::Id) -> Span {
        Span
    }

    pub fn interact(_window: window::Id) -> Span {
        Span
    }

    pub fn draw(_window: window::Id) -> Span {
        Span
    }

    pub fn prepare(_primitive: Primitive) -> Span {
        Span
    }

    pub fn render(_primitive: Primitive) -> Span {
        Span
    }

    pub fn present(_window: window::Id) -> Span {
        Span
    }

    pub fn time(_name: impl Into<String>) -> Span {
        Span
    }

    pub fn commands() -> Subscription<Command> {
        Subscription::none()
    }

    #[derive(Debug)]
    pub struct Span;

    impl Span {
        pub fn finish(self) {}
    }
}

#[cfg(feature = "hot")]
mod hot {
    use std::collections::BTreeSet;
    use std::sync::atomic::{self, AtomicBool};
    use std::sync::{Arc, Mutex, OnceLock};

    #[cfg(target_arch = "wasm32")]
    use serde::Deserialize;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen::{JsCast, closure::Closure};

    #[cfg(target_arch = "wasm32")]
    use web_sys::{CloseEvent, MessageEvent, WebSocket};

    static IS_STALE: AtomicBool = AtomicBool::new(false);

    static HOT_FUNCTIONS_PENDING: Mutex<BTreeSet<u64>> =
        Mutex::new(BTreeSet::new());

    static HOT_FUNCTIONS: OnceLock<BTreeSet<u64>> = OnceLock::new();

    pub fn init() {
        subsecond::register_handler(Arc::new(|| {
            if HOT_FUNCTIONS.get().is_none() {
                HOT_FUNCTIONS
                    .set(std::mem::take(
                        &mut HOT_FUNCTIONS_PENDING
                            .lock()
                            .expect("Lock hot functions"),
                    ))
                    .expect("Set hot functions");
            }

            IS_STALE.store(false, atomic::Ordering::Relaxed);
        }));

        #[cfg(target_arch = "wasm32")]
        connect_to_devserver();
    }

    #[cfg(target_arch = "wasm32")]
    #[derive(Deserialize)]
    enum DevserverMessage {
        HotReload(HotReloadMessage),
        HotPatchStart,
        FullReloadStart,
        FullReloadFailed,
        FullReloadCommand,
        Shutdown,
    }

    #[cfg(target_arch = "wasm32")]
    #[derive(Deserialize)]
    struct HotReloadMessage {
        jump_table: Option<subsecond::JumpTable>,
        for_build_id: Option<u64>,
        for_pid: Option<u32>,
    }

    #[cfg(target_arch = "wasm32")]
    fn connect_to_devserver() {
        const RECONNECT_DELAY_MS: i32 = 250;

        let Some(window) = web_sys::window() else {
            return;
        };
        let location = window.location();
        let Ok(host) = location.host() else {
            return;
        };
        let protocol = match location.protocol().as_deref() {
            Ok("https:") => "wss:",
            _ => "ws:",
        };
        let Ok(websocket) =
            WebSocket::new(&format!("{protocol}//{host}/_dioxus?build_id=0"))
        else {
            return;
        };

        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(
            move |event: MessageEvent| {
                let Some(message) = event.data().as_string() else {
                    return;
                };
                let Ok(message) =
                    serde_json::from_str::<DevserverMessage>(&message)
                else {
                    return;
                };

                match message {
                    DevserverMessage::HotReload(message)
                        if message.for_build_id == Some(0)
                            && message.for_pid.is_none() =>
                    {
                        if let Some(jump_table) = message.jump_table {
                            // The jump table comes from the local Dioxus devserver.
                            unsafe {
                                subsecond::apply_patch(jump_table)
                                    .expect("apply WASM hot patch");
                            }
                        }
                    }
                    DevserverMessage::FullReloadCommand => {
                        let _ = web_sys::window()
                            .expect("browser window")
                            .location()
                            .reload();
                    }
                    _ => {}
                }
            },
        );
        websocket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();

        let on_close =
            Closure::<dyn FnMut(CloseEvent)>::new(move |event: CloseEvent| {
                if event.code() == 1001 {
                    return;
                }

                let reconnect =
                    Closure::<dyn FnMut()>::once(connect_to_devserver);
                let _ = web_sys::window()
                    .expect("browser window")
                    .set_timeout_with_callback_and_timeout_and_arguments_0(
                        reconnect.as_ref().unchecked_ref(),
                        RECONNECT_DELAY_MS,
                    );
                reconnect.forget();
            });
        websocket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        on_close.forget();
    }

    pub fn call<O>(f: impl FnOnce() -> O) -> O {
        let mut f = Some(f);

        // The `move` here is important. Hotpatching will not work
        // otherwise.
        let mut f = subsecond::HotFn::current(move || {
            f.take().expect("Hot function is stale")()
        });

        let address = f.ptr_address().0;

        if let Some(hot_functions) = HOT_FUNCTIONS.get() {
            if hot_functions.contains(&address) {
                IS_STALE.store(true, atomic::Ordering::Relaxed);
            }
        } else {
            let _ = HOT_FUNCTIONS_PENDING
                .lock()
                .expect("Lock hot functions")
                .insert(address);
        }

        f.call(())
    }

    pub fn on_hotpatch(f: impl Fn() + Send + Sync + 'static) {
        subsecond::register_handler(Arc::new(f));
    }

    pub fn is_stale() -> bool {
        IS_STALE.load(atomic::Ordering::Relaxed)
    }
}

#[cfg(not(feature = "hot"))]
mod hot {
    pub fn init() {}

    pub fn call<O>(f: impl FnOnce() -> O) -> O {
        f()
    }

    pub fn on_hotpatch(_f: impl Fn()) {}

    pub fn is_stale() -> bool {
        false
    }
}
