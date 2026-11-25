mod app;
mod app_icon;
#[cfg(target_os = "macos")]
mod appkit;
mod areas;
mod debug;
mod glass;
mod icons;
mod menus;
mod overlay;
mod ui;
mod worker;

use std::time::Duration;

use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, point, px, size,
};
use gpui_component::Root;
use tokio::sync::mpsc::unbounded_channel;

use app::OpenMusic;
use worker::Command;

fn main() {
    let initial_track = std::env::args().nth(1);
    let runtime = tokio::runtime::Runtime::new().expect("start OpenMusic background runtime");
    let (commands, inbound) = unbounded_channel();
    let (outbound, mut events) = unbounded_channel();
    runtime.spawn(worker::run(inbound, outbound));

    Application::new()
        .with_assets(icons::Icons)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            app_icon::install();
            menus::install(cx);
            areas::system::state::install(cx);
            let bounds = Bounds::centered(None, size(px(1120.), px(760.)), cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("OpenMusic".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(22.), px(22.))),
                    }),
                    window_background: WindowBackgroundAppearance::Transparent,
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(800.), px(600.))),
                    ..Default::default()
                },
                |window, cx| {
                    if !glass::install(window) {
                        window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                    }
                    ui::apply_theme(window.appearance(), cx);
                    let (controls, mut presses) = unbounded_channel();
                    overlay::set_handler(overlay::handler(controls));
                    let player = cx.new(|cx| OpenMusic::new(commands, initial_track, window, cx));
                    menus::connect(player.downgrade(), window.window_handle(), cx);
                    areas::system::state::window_opened(window, cx);
                    window.focus(&player.read(cx).focus);
                    player.update(cx, |this, cx| {
                        this.send(Command::Restore);
                        cx.spawn_in(window, async move |view, window| {
                            while let Some(action) = presses.recv().await {
                                if view
                                    .update_in(window, |this, window, cx| {
                                        this.handle_control(action, window, cx);
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                        cx.spawn_in(window, async move |view, window| {
                            while let Some(event) = events.recv().await {
                                if view
                                    .update_in(window, |this, window, cx| {
                                        this.handle_event(event, window, cx);
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                        cx.spawn_in(window, async move |view, window| {
                            loop {
                                window
                                    .background_executor()
                                    .timer(Duration::from_millis(250))
                                    .await;
                                if view
                                    .update_in(window, |this, window, cx| this.tick(window, cx))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                    });
                    if let Some(directory) = debug::directory() {
                        player.update(cx, |_, cx| {
                            cx.spawn_in(window, async move |view, window| {
                                let command_file = directory.join("command");
                                loop {
                                    window
                                        .background_executor()
                                        .timer(Duration::from_millis(250))
                                        .await;
                                    let Ok(commands) = std::fs::read_to_string(&command_file)
                                    else {
                                        continue;
                                    };
                                    let _ = std::fs::remove_file(&command_file);
                                    for line in commands
                                        .lines()
                                        .map(str::trim)
                                        .filter(|line| !line.is_empty())
                                    {
                                        let line = line.to_owned();
                                        let directory = directory.clone();
                                        if view
                                            .update_in(window, |this, window, cx| {
                                                this.debug_command(&line, &directory, window, cx)
                                            })
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                }
                            })
                            .detach();
                        });
                    }
                    cx.new(|cx| Root::new(player, window, cx))
                },
            )
            .expect("open the OpenMusic window");
            if debug::directory().is_none() {
                cx.activate(true);
            }
        });

    runtime.shutdown_timeout(Duration::from_secs(1));
}
