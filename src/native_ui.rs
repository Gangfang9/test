use bevy::prelude::*;
use bevy::window::WindowLevel;
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::fs;

use crate::{
    config::LocalConfig,
    mask::ui::basic::{ProjectionBodyMarker, TITLEBAR_HEIGHT},
    scrcpy::adb::Device,
    tokio_tasks::TokioTasksRuntime,
    utils::{ChannelSenderD, ChannelSenderWS, relate_to_data_path},
    web::device::{list_usb_devices, restart_adb_and_list_usb_devices, start_usb_device},
};

const BG: Color = Color::srgb(0.035, 0.035, 0.04);
const PANEL: Color = Color::srgb(0.09, 0.095, 0.105);
const PANEL_ALT: Color = Color::srgb(0.12, 0.125, 0.135);
const BORDER: Color = Color::srgb(0.22, 0.225, 0.24);
const TEXT: Color = Color::srgb(0.92, 0.92, 0.94);
const MUTED: Color = Color::srgb(0.62, 0.64, 0.68);
const ACCENT: Color = Color::srgb(0.78, 0.12, 0.09);
const ACCENT_HOVER: Color = Color::srgb(0.92, 0.18, 0.13);
const INFO: Color = Color::srgb(0.045, 0.11, 0.22);

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
enum NativePage {
    #[default]
    Device,
    Mapping,
    Settings,
}

#[derive(Component)]
struct NativeDevicePage;

#[derive(Component)]
struct NativeMappingPage;

#[derive(Component)]
struct NativeSettingsPage;

#[derive(Component)]
struct NativeNavButton(NativePage);

#[derive(Component)]
struct MappingFileButton(String);

#[derive(Component)]
struct NativePageStatus;

#[derive(Component)]
struct ToggleAlwaysOnTopButton;

#[derive(Component)]
struct ToggleTitlebarButton;

#[derive(Component)]
struct CycleOrientationButton;

#[derive(Component)]
pub struct NativeDashboardRoot;

#[derive(Component)]
struct RefreshButton;

#[derive(Component)]
struct RestartAdbButton;

#[derive(Component)]
struct StartProjectionButton;

#[derive(Component)]
struct DeviceIdentityText;

#[derive(Component)]
struct DeviceStatusText;

#[derive(Component)]
struct FooterStatusText;

#[derive(Resource, Default)]
struct NativeDeviceState {
    devices: Vec<Device>,
    busy: bool,
    status: String,
}

enum NativeUiResult {
    Devices(Result<Vec<Device>, String>),
    Projection(Result<(), String>),
}

#[derive(Resource)]
struct NativeUiChannel {
    tx: Sender<NativeUiResult>,
    rx: Receiver<NativeUiResult>,
}

impl Default for NativeUiChannel {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self { tx, rx }
    }
}

pub struct NativeUiPlugin;

impl Plugin for NativeUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NativeDeviceState>()
            .init_resource::<NativePage>()
            .init_resource::<NativeUiChannel>()
            .add_systems(PostStartup, setup_native_dashboard)
            .add_systems(
                Update,
                (
                    handle_native_buttons,
                    handle_native_navigation,
                    receive_native_results,
                    sync_native_dashboard,
                    native_button_style,
                    native_navigation_style,
                ),
            );
    }
}

fn ui_text(font: Handle<Font>, size: f32, color: Color) -> (TextFont, TextColor) {
    (
        TextFont {
            font: font.into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn setup_native_dashboard(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut window: Single<&mut Window>,
    channel: Res<NativeUiChannel>,
) {
    window.title = "JX手游助手".to_string();
    window.resolution.set(1280., 760. + TITLEBAR_HEIGHT);
    window.visible = true;
    window.focused = true;

    let font = asset_server.load("fonts/NotoSansSC-Regular.otf");
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(TITLEBAR_HEIGHT),
                left: Val::Px(0.),
                right: Val::Px(0.),
                bottom: Val::Px(0.),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(BG),
            NativeDashboardRoot,
        ))
        .id();

    commands.entity(root).with_children(|root| {
        root.spawn(Node {
            width: Val::Percent(100.),
            flex_grow: 1.,
            flex_direction: FlexDirection::Row,
            ..default()
        })
        .with_children(|layout| {
            layout
                .spawn((
                    Node {
                        width: Val::Px(180.),
                        height: Val::Percent(100.),
                        padding: UiRect::all(Val::Px(12.)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.055, 0.058, 0.065)),
                    BorderColor::all(BORDER),
                ))
                .with_children(|sidebar| {
                    spawn_nav_item(sidebar, font.clone(), "▣   设备", true, NativePage::Device);
                    spawn_nav_item(sidebar, font.clone(), "⌨   编辑映射", false, NativePage::Mapping);
                    spawn_nav_item(sidebar, font.clone(), "⚙   设置", false, NativePage::Settings);
                });

            layout
                    .spawn((
                        Node {
                            flex_grow: 1.,
                            height: Val::Percent(100.),
                            padding: UiRect::all(Val::Px(24.)),
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(20.),
                            ..default()
                        },
                        NativeDevicePage,
                    ))
                .with_children(|content| {
                    content
                        .spawn(Node {
                            flex_grow: 1.,
                            height: Val::Percent(100.),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(16.),
                            ..default()
                        })
                        .with_children(|main| {
                            main.spawn((Text::new("USB设备连接"), ui_text(font.clone(), 30., TEXT)));

                            main.spawn((
                                Node {
                                    width: Val::Percent(100.),
                                    padding: UiRect::axes(Val::Px(18.), Val::Px(14.)),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(4.),
                                    border: UiRect::all(Val::Px(1.)),
                                    border_radius: BorderRadius::all(Val::Px(6.)),
                                    ..default()
                                },
                                BackgroundColor(INFO),
                                BorderColor::all(Color::srgb(0.08, 0.34, 0.68)),
                            ))
                            .with_children(|info| {
                                info.spawn((
                                    Text::new("第一版支持一台 USB ADB 设备"),
                                    ui_text(font.clone(), 17., TEXT),
                                ));
                                info.spawn((
                                    Text::new("请在手机上开启 USB 调试并授权此电脑"),
                                    ui_text(font.clone(), 13., MUTED),
                                ));
                            });

                            main.spawn(Node {
                                flex_direction: FlexDirection::Row,
                                column_gap: Val::Px(10.),
                                ..default()
                            })
                            .with_children(|toolbar| {
                                spawn_action_button(
                                    toolbar,
                                    font.clone(),
                                    "重启 ADB",
                                    false,
                                    RestartAdbButton,
                                );
                                spawn_action_button(
                                    toolbar,
                                    font.clone(),
                                    "刷新设备",
                                    true,
                                    RefreshButton,
                                );
                            });

                            main.spawn((
                                Node {
                                    width: Val::Percent(100.),
                                    flex_grow: 1.,
                                    padding: UiRect::all(Val::Px(16.)),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(12.),
                                    border: UiRect::all(Val::Px(1.)),
                                    border_radius: BorderRadius::all(Val::Px(6.)),
                                    ..default()
                                },
                                BackgroundColor(PANEL),
                                BorderColor::all(BORDER),
                            ))
                            .with_children(|card| {
                                card.spawn((Text::new("受控设备"), ui_text(font.clone(), 20., TEXT)));
                                card.spawn((
                                    Node {
                                        width: Val::Percent(100.),
                                        height: Val::Px(44.),
                                        padding: UiRect::horizontal(Val::Px(14.)),
                                        align_items: AlignItems::Center,
                                        ..default()
                                    },
                                    BackgroundColor(PANEL_ALT),
                                ))
                                .with_children(|header| {
                                    spawn_table_text(header, font.clone(), "身份码", 2.);
                                    spawn_table_text(header, font.clone(), "状态", 1.);
                                    spawn_table_text(header, font.clone(), "操作", 1.);
                                });
                                card.spawn((
                                    Node {
                                        width: Val::Percent(100.),
                                        height: Val::Px(58.),
                                        padding: UiRect::horizontal(Val::Px(14.)),
                                        align_items: AlignItems::Center,
                                        border: UiRect::bottom(Val::Px(1.)),
                                        ..default()
                                    },
                                    BorderColor::all(BORDER),
                                ))
                                .with_children(|row| {
                                    row.spawn((
                                        Text::new("正在检测设备…"),
                                        ui_text(font.clone(), 15., TEXT),
                                        Node { flex_grow: 2., ..default() },
                                        DeviceIdentityText,
                                    ));
                                    row.spawn((
                                        Text::new("检测中"),
                                        ui_text(font.clone(), 14., MUTED),
                                        Node { flex_grow: 1., ..default() },
                                        DeviceStatusText,
                                    ));
                                    spawn_action_button(
                                        row,
                                        font.clone(),
                                        "投屏",
                                        true,
                                        StartProjectionButton,
                                    );
                                });
                            });
                        });

                    content
                        .spawn((
                            Node {
                                width: Val::Px(300.),
                                height: Val::Percent(100.),
                                padding: UiRect::all(Val::Px(18.)),
                                flex_direction: FlexDirection::Column,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                row_gap: Val::Px(14.),
                                border: UiRect::all(Val::Px(1.)),
                                border_radius: BorderRadius::all(Val::Px(6.)),
                                ..default()
                            },
                            BackgroundColor(PANEL),
                            BorderColor::all(BORDER),
                        ))
                        .with_children(|preview| {
                            preview.spawn((
                                Node {
                                    width: Val::Px(210.),
                                    height: Val::Px(420.),
                                    border: UiRect::all(Val::Px(2.)),
                                    border_radius: BorderRadius::all(Val::Px(18.)),
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.025, 0.028, 0.035)),
                                BorderColor::all(Color::srgb(0.35, 0.37, 0.42)),
                            ))
                            .with_child((
                                Text::new("连接设备后\n点击“投屏”"),
                                ui_text(font.clone(), 16., MUTED),
                                TextLayout::justify(Justify::Center),
                            ));
                            preview.spawn((
                                Text::new("USB  •  ADB"),
                                ui_text(font.clone(), 13., MUTED),
                            ));
                        });
                });

            spawn_mapping_page(layout, font.clone());
            spawn_settings_page(layout, font.clone());
        });

        root.spawn((
            Node {
                width: Val::Percent(100.),
                height: Val::Px(34.),
                padding: UiRect::horizontal(Val::Px(18.)),
                align_items: AlignItems::Center,
                border: UiRect::top(Val::Px(1.)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.055, 0.058, 0.065)),
            BorderColor::all(BORDER),
        ))
        .with_child((
            Text::new("●  正在检测 ADB 设备"),
            ui_text(font, 13., MUTED),
            FooterStatusText,
        ));
    });

    let tx = channel.tx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(NativeUiResult::Devices(list_usb_devices()));
    });
}

fn spawn_nav_item(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    label: &str,
    active: bool,
    page: NativePage,
) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Percent(100.),
                height: Val::Px(48.),
                padding: UiRect::horizontal(Val::Px(14.)),
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(6.)),
                ..default()
            },
            BackgroundColor(if active {
                Color::srgb(0.25, 0.07, 0.06)
            } else {
                Color::NONE
            }),
            NativeNavButton(page),
        ))
        .with_child((
            Text::new(label),
            ui_text(font, 16., if active { TEXT } else { MUTED }),
        ));
}

fn spawn_mapping_page(parent: &mut ChildSpawnerCommands, font: Handle<Font>) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.),
                height: Val::Percent(100.),
                padding: UiRect::all(Val::Px(24.)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(14.),
                display: Display::None,
                ..default()
            },
            NativeMappingPage,
        ))
        .with_children(|page| {
            page.spawn((Text::new("编辑映射"), ui_text(font.clone(), 30., TEXT)));
            page.spawn((
                Text::new("选择映射配置后，可在投屏窗口中直接使用对应按键。"),
                ui_text(font.clone(), 14., MUTED),
            ));
            page.spawn((
                Node {
                    width: Val::Percent(100.),
                    flex_grow: 1.,
                    padding: UiRect::all(Val::Px(16.)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.),
                    border: UiRect::all(Val::Px(1.)),
                    border_radius: BorderRadius::all(Val::Px(6.)),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(BORDER),
            ))
            .with_children(|card| {
                card.spawn((Text::new("映射配置"), ui_text(font.clone(), 19., TEXT)));
                let active = LocalConfig::get().active_mapping_file;
                let mapping_dir = relate_to_data_path(["mapping"]);
                let mut files: Vec<String> = fs::read_dir(mapping_dir)
                    .ok()
                    .into_iter()
                    .flat_map(|entries| entries.flatten())
                    .filter_map(|entry| {
                        let path = entry.path();
                        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                            path.file_name().map(|name| name.to_string_lossy().into_owned())
                        } else {
                            None
                        }
                    })
                    .collect();
                files.sort();
                if files.is_empty() {
                    card.spawn((Text::new("未找到映射配置文件"), ui_text(font.clone(), 14., MUTED)));
                } else {
                    for file in files {
                        let is_active = file == active;
                        card.spawn((
                            Button,
                            Node {
                                width: Val::Percent(100.),
                                min_height: Val::Px(42.),
                                padding: UiRect::horizontal(Val::Px(14.)),
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.)),
                                border_radius: BorderRadius::all(Val::Px(5.)),
                                ..default()
                            },
                            BackgroundColor(if is_active { ACCENT } else { PANEL_ALT }),
                            BorderColor::all(if is_active { ACCENT } else { BORDER }),
                            MappingFileButton(file.clone()),
                        ))
                        .with_child((Text::new(if is_active { format!("✓  {file}") } else { file }), ui_text(font.clone(), 14., TEXT)));
                    }
                }
                card.spawn((
                    Text::new("提示：映射文件可在 data/mapping 目录中维护；选择后下次投屏生效。"),
                    ui_text(font.clone(), 13., MUTED),
                ));
            });
            page.spawn((Text::new("点击左侧“设备”返回设备控制。"), ui_text(font, 13., MUTED)));
        });
}

fn spawn_settings_page(parent: &mut ChildSpawnerCommands, font: Handle<Font>) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.),
                height: Val::Percent(100.),
                padding: UiRect::all(Val::Px(24.)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(14.),
                display: Display::None,
                ..default()
            },
            NativeSettingsPage,
        ))
        .with_children(|page| {
            page.spawn((Text::new("设置"), ui_text(font.clone(), 30., TEXT)));
            page.spawn((Text::new("投屏和窗口行为"), ui_text(font.clone(), 16., MUTED)));
            page.spawn((
                Node {
                    width: Val::Percent(100.),
                    padding: UiRect::all(Val::Px(16.)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.),
                    border: UiRect::all(Val::Px(1.)),
                    border_radius: BorderRadius::all(Val::Px(6.)),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(BORDER),
            ))
            .with_children(|card| {
                spawn_setting_button(card, font.clone(), "切换置顶", ToggleAlwaysOnTopButton);
                spawn_setting_button(card, font.clone(), "切换标题栏", ToggleTitlebarButton);
                spawn_setting_button(card, font.clone(), "旋转投屏画面", CycleOrientationButton);
                card.spawn((Text::new("当前设置"), ui_text(font.clone(), 16., TEXT)));
                let config = LocalConfig::get();
                card.spawn((
                    Text::new(format!(
                        "置顶：{}    标题栏：{}    投屏方向：{}°    当前映射：{}",
                        if config.always_on_top { "开" } else { "关" },
                        if config.titlebar_visible { "开" } else { "关" },
                        if config.capture_orientation < 0 { "跟随设备".to_string() } else { config.capture_orientation.to_string() },
                        config.active_mapping_file,
                    )),
                    ui_text(font.clone(), 14., MUTED),
                    NativePageStatus,
                ));
            });
        });
}

fn spawn_setting_button<M: Component>(parent: &mut ChildSpawnerCommands, font: Handle<Font>, label: &str, marker: M) {
    parent.spawn((
        Button,
        Node {
            width: Val::Px(190.),
            height: Val::Px(38.),
            padding: UiRect::horizontal(Val::Px(14.)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::all(Val::Px(1.)),
            border_radius: BorderRadius::all(Val::Px(5.)),
            ..default()
        },
        BackgroundColor(PANEL_ALT),
        BorderColor::all(BORDER),
        marker,
    )).with_child((Text::new(label), ui_text(font, 14., TEXT)));
}

fn spawn_action_button<M: Component>(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    label: &str,
    primary: bool,
    marker: M,
) {
    parent
        .spawn((
            Button,
            Node {
                min_width: Val::Px(92.),
                height: Val::Px(38.),
                padding: UiRect::horizontal(Val::Px(16.)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(Val::Px(1.)),
                border_radius: BorderRadius::all(Val::Px(5.)),
                ..default()
            },
            BackgroundColor(if primary { ACCENT } else { PANEL_ALT }),
            BorderColor::all(if primary { ACCENT } else { BORDER }),
            marker,
        ))
        .with_child((Text::new(label), ui_text(font, 14., TEXT)));
}

fn spawn_table_text(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    value: &str,
    grow: f32,
) {
    parent.spawn((
        Text::new(value),
        ui_text(font, 13., MUTED),
        Node { flex_grow: grow, ..default() },
    ));
}

fn handle_native_buttons(
    refresh: Query<&Interaction, (With<RefreshButton>, Changed<Interaction>)>,
    restart: Query<&Interaction, (With<RestartAdbButton>, Changed<Interaction>)>,
    start: Query<&Interaction, (With<StartProjectionButton>, Changed<Interaction>)>,
    mut state: ResMut<NativeDeviceState>,
    channel: Res<NativeUiChannel>,
    runtime: Res<TokioTasksRuntime>,
    d_tx: Res<ChannelSenderD>,
    ws_tx: Res<ChannelSenderWS>,
) {
    if state.busy {
        return;
    }
    if refresh.iter().any(|interaction| *interaction == Interaction::Pressed) {
        state.busy = true;
        state.status = "正在刷新设备…".to_string();
        let tx = channel.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(NativeUiResult::Devices(list_usb_devices()));
        });
    }
    if restart.iter().any(|interaction| *interaction == Interaction::Pressed) {
        state.busy = true;
        state.status = "正在重启 ADB…".to_string();
        let tx = channel.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(NativeUiResult::Devices(restart_adb_and_list_usb_devices()));
        });
    }
    if start.iter().any(|interaction| *interaction == Interaction::Pressed) {
        let Some(device_id) = state.devices.first().map(|device| device.id.clone()) else {
            state.status = "没有可投屏的 USB ADB 设备".to_string();
            return;
        };
        state.busy = true;
        state.status = "正在启动投屏…".to_string();
        let d_tx = d_tx.0.clone();
        let ws_tx = ws_tx.0.clone();
        let tx = channel.tx.clone();
        runtime.spawn_background_task(move |_ctx| async move {
            let result = start_usb_device(&device_id, &d_tx, &ws_tx).await;
            let _ = tx.send(NativeUiResult::Projection(result));
        });
    }
}

fn handle_native_navigation(
    nav: Query<(&Interaction, &NativeNavButton), Changed<Interaction>>,
    mapping: Query<(&Interaction, &MappingFileButton), Changed<Interaction>>,
    always_on_top: Query<&Interaction, (With<ToggleAlwaysOnTopButton>, Changed<Interaction>)>,
    titlebar: Query<&Interaction, (With<ToggleTitlebarButton>, Changed<Interaction>)>,
    orientation: Query<&Interaction, (With<CycleOrientationButton>, Changed<Interaction>)>,
    mut page: ResMut<NativePage>,
    mut window: Single<&mut Window>,
    mut device_page: Query<&mut Node, With<NativeDevicePage>>,
    mut mapping_page: Query<&mut Node, (With<NativeMappingPage>, Without<NativeDevicePage>)>,
    mut settings_page: Query<&mut Node, (With<NativeSettingsPage>, Without<NativeDevicePage>)>,
    mut status: Query<&mut Text, With<NativePageStatus>>,
) {
    if let Some((_, button)) = nav.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) {
        *page = button.0;
    }
    if let Some((_, file)) = mapping.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) {
        LocalConfig::set_active_mapping_file(file.0.clone());
        for mut text in status.iter_mut() {
            text.0 = format!("已选择映射：{}（下次投屏生效）", file.0);
        }
    }
    if always_on_top.iter().any(|interaction| *interaction == Interaction::Pressed) {
        let value = !LocalConfig::get().always_on_top;
        LocalConfig::set_always_on_top(value);
        window.window_level = if value { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal };
        for mut text in status.iter_mut() {
            text.0 = format!("置顶已{}", if value { "开启" } else { "关闭" });
        }
    }
    if titlebar.iter().any(|interaction| *interaction == Interaction::Pressed) {
        let value = !LocalConfig::get().titlebar_visible;
        LocalConfig::set_titlebar_visible(value);
        for mut text in status.iter_mut() {
            text.0 = format!("标题栏已{}（下次打开投屏窗口生效）", if value { "显示" } else { "隐藏" });
        }
    }
    if orientation.iter().any(|interaction| *interaction == Interaction::Pressed) {
        let next = match LocalConfig::get().capture_orientation {
            -1 => 0,
            0 => 90,
            90 => 180,
            180 => 270,
            _ => -1,
        };
        LocalConfig::set_capture_orientation(next);
        for mut text in status.iter_mut() {
            text.0 = if next < 0 { "投屏方向：跟随设备".to_string() } else { format!("投屏方向：{next}°") };
        }
    }

    for mut node in device_page.iter_mut() {
        node.display = if *page == NativePage::Device { Display::Flex } else { Display::None };
    }
    for mut node in mapping_page.iter_mut() {
        node.display = if *page == NativePage::Mapping { Display::Flex } else { Display::None };
    }
    for mut node in settings_page.iter_mut() {
        node.display = if *page == NativePage::Settings { Display::Flex } else { Display::None };
    }
}

fn receive_native_results(
    channel: Res<NativeUiChannel>,
    mut state: ResMut<NativeDeviceState>,
    mut dashboard: Query<&mut Node, With<NativeDashboardRoot>>,
    mut projection: Query<&mut Node, (With<ProjectionBodyMarker>, Without<NativeDashboardRoot>)>,
) {
    for result in channel.rx.try_iter() {
        state.busy = false;
        match result {
            NativeUiResult::Devices(Ok(devices)) => {
                state.devices = devices;
                state.status = if state.devices.is_empty() {
                    "未检测到已授权的 USB ADB 设备".to_string()
                } else {
                    format!("ADB 已连接：{}", state.devices[0].id)
                };
            }
            NativeUiResult::Devices(Err(error)) => {
                state.devices.clear();
                state.status = format!("ADB 错误：{error}");
            }
            NativeUiResult::Projection(Ok(())) => {
                state.status = "投屏已启动".to_string();
                for mut node in dashboard.iter_mut() {
                    node.display = Display::None;
                }
                for mut node in projection.iter_mut() {
                    node.display = Display::Flex;
                }
            }
            NativeUiResult::Projection(Err(error)) => {
                state.status = format!("投屏失败：{error}");
            }
        }
    }
}

fn sync_native_dashboard(
    state: Res<NativeDeviceState>,
    mut identities: Query<&mut Text, With<DeviceIdentityText>>,
    mut statuses: Query<&mut Text, (With<DeviceStatusText>, Without<DeviceIdentityText>)>,
    mut footer: Query<
        &mut Text,
        (
            With<FooterStatusText>,
            Without<DeviceStatusText>,
            Without<DeviceIdentityText>,
        ),
    >,
) {
    if !state.is_changed() {
        return;
    }
    let identity = state
        .devices
        .first()
        .map(|device| device.id.as_str())
        .unwrap_or("没有数据");
    let device_status = state
        .devices
        .first()
        .map(|device| if device.status == "device" { "●  已连接" } else { "●  未授权" })
        .unwrap_or("未连接");
    for mut text in identities.iter_mut() {
        text.0 = identity.to_string();
    }
    for mut text in statuses.iter_mut() {
        text.0 = device_status.to_string();
    }
    for mut text in footer.iter_mut() {
        text.0 = if state.status.is_empty() {
            "●  ADB 就绪".to_string()
        } else {
            state.status.clone()
        };
    }
}

fn native_button_style(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (
            Changed<Interaction>,
            Or<(With<RefreshButton>, With<RestartAdbButton>, With<StartProjectionButton>)>,
        ),
    >,
) {
    for (interaction, mut background) in buttons.iter_mut() {
        *background = match *interaction {
            Interaction::Pressed => Color::srgb(0.42, 0.06, 0.045),
            Interaction::Hovered => ACCENT_HOVER,
            Interaction::None => ACCENT,
        }
        .into();
    }
}

fn native_navigation_style(
    page: Res<NativePage>,
    mut nav: Query<(&NativeNavButton, &Interaction, &mut BackgroundColor)>,
    mut controls: Query<
        (&Interaction, &mut BackgroundColor),
        Or<(
            With<MappingFileButton>,
            With<ToggleAlwaysOnTopButton>,
            With<ToggleTitlebarButton>,
            With<CycleOrientationButton>,
        )>,
    >,
) {
    for (button, interaction, mut background) in nav.iter_mut() {
        *background = if button.0 == *page {
            ACCENT.into()
        } else {
            match *interaction {
                Interaction::Hovered => PANEL_ALT.into(),
                Interaction::Pressed => Color::srgb(0.2, 0.05, 0.04).into(),
                Interaction::None => Color::NONE.into(),
            }
        };
    }
    for (interaction, mut background) in controls.iter_mut() {
        *background = match *interaction {
            Interaction::Hovered => ACCENT_HOVER.into(),
            Interaction::Pressed => Color::srgb(0.42, 0.06, 0.045).into(),
            Interaction::None => PANEL_ALT.into(),
        };
    }
}
