use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    input::{ButtonState, keyboard::KeyboardInput, mouse::{MouseScrollUnit, MouseWheel}},
    prelude::*,
};
use bevy::window::WindowLevel;
use crossbeam_channel::{Receiver, Sender, unbounded};
use serde_json::{Value, json};
use std::fs;

use crate::{
    config::LocalConfig,
    mask::{mask_command::MaskCommand, ui::basic::{ProjectionBodyMarker, TITLEBAR_HEIGHT}},
    mask::mapping::{binding::MergedButton, config::{MappingConfig, validate_mapping_config_diagnostics}},
    scrcpy::adb::Device,
    scrcpy::media::{AudioCodec, VideoCodec},
    tokio_tasks::TokioTasksRuntime,
    utils::{ChannelSenderD, ChannelSenderM, ChannelSenderWS, relate_to_data_path},
    web::device::{capture_adb_screenshot, list_usb_devices, restart_adb_and_list_usb_devices, start_usb_device},
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
const NAV_ACTIVE: Color = Color::srgb(0.29, 0.075, 0.065);
const NAV_ACTIVE_TEXT: Color = Color::srgb(0.95, 0.25, 0.20);
const EDITOR_CANVAS_WIDTH: f32 = 790.;
const EDITOR_CANVAS_HEIGHT: f32 = 444.;

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
struct NativeNavLabel(NativePage);

#[derive(Component)]
struct MappingFileButton(String);

#[derive(Component)]
struct NativeMappingCanvas;

#[derive(Component)]
struct NativeMappingNode(usize);

#[derive(Component)]
struct NativeMappingNodeLabel(usize);

#[derive(Component)]
struct NativeMappingGuide(usize);

#[derive(Component, Clone, Copy)]
struct NativeMappingToolbarButton(NativeMappingToolbarAction);

#[derive(Component, Clone, Copy)]
struct NativeMappingPropertyButton(NativeMappingPropertyAction);

#[derive(Component, Clone)]
struct NativeAddMappingButton(&'static str);

#[derive(Component)]
struct NativeMappingEditorStatus;

#[derive(Component)]
struct NativeMappingInspectorValue;

#[derive(Component)]
struct NativeMappingInspectorScroll;

#[derive(Component)]
struct NativeMappingFileText;

#[derive(Component)]
struct NativeMappingBackground;

#[derive(Component)]
struct NativeAdvancedEditorOverlay;

#[derive(Component)]
struct NativeAdvancedEditorText;

#[derive(Component)]
struct NativeAdvancedEditorButton(bool);

#[derive(Component, Clone, Copy)]
struct NativeMappingManageButton(NativeMappingManageAction);

#[derive(Clone, Copy)]
enum NativeMappingToolbarAction {
    Save,
    Restore,
    Activate,
    Refresh,
    ToggleGuides,
    RefreshBackground,
}

#[derive(Clone, Copy)]
enum NativeMappingPropertyAction {
    SizeDown,
    SizeUp,
    RandomXDown,
    RandomXUp,
    RandomYDown,
    RandomYUp,
    CycleRandomAlgorithm,
    CaptureBinding,
    BindMouseLeft,
    AdvancedEdit,
    Delete,
}

#[derive(Clone, Copy)]
enum NativeMappingManageAction {
    Create,
    Duplicate,
    Rename,
    Delete,
    OpenFolder,
}

#[derive(Resource)]
struct NativeMappingEditorState {
    file: String,
    current: Value,
    original: Value,
    selected: Option<usize>,
    dragging: Option<usize>,
    dirty: bool,
    show_guides: bool,
    needs_rebuild: bool,
    capturing_binding: bool,
    advanced_edit: Option<(usize, String)>,
    status: String,
}

impl Default for NativeMappingEditorState {
    fn default() -> Self {
        let file = LocalConfig::get().active_mapping_file;
        let current = read_mapping_value(&file).unwrap_or_else(|_| empty_mapping_value());
        Self {
            file,
            original: current.clone(),
            current,
            selected: None,
            dragging: None,
            dirty: false,
            show_guides: true,
            needs_rebuild: false,
            capturing_binding: false,
            advanced_edit: None,
            status: "拖动按键可调整位置；右侧可调整大小与随机偏移。".to_string(),
        }
    }
}

#[derive(Component)]
struct NativePageStatus;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct NativeSettingButton(NativeSetting);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct NativeSettingValue(NativeSetting);

#[derive(Clone, Copy, PartialEq, Eq)]
enum NativeSetting {
    VideoMaxSize,
    VideoMaxFps,
    VideoBitRate,
    VideoCodec,
    CaptureOrientation,
    AudioEnabled,
    AudioCodec,
    AudioBitRate,
    AlwaysOnTop,
    TitlebarVisible,
    MappingOpacity,
    StayAwake,
    ClipboardSync,
}

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
    MappingActivated { file: String, result: Result<String, String> },
    MappingBackground(Result<Vec<u8>, String>),
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
            .init_resource::<NativeMappingEditorState>()
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
                    handle_native_settings,
                    sync_native_setting_values,
                    handle_native_mapping_toolbar,
                    handle_native_mapping_selection_and_drag,
                    handle_native_mapping_properties,
                    capture_native_mapping_binding,
                    handle_native_advanced_editor,
                    sync_native_advanced_editor,
                    handle_native_add_mapping,
                    handle_native_mapping_management,
                    rebuild_native_mapping_nodes,
                    sync_native_mapping_editor,
                    sync_native_mapping_node_labels,
                    scroll_native_mapping_inspector,
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
                        padding: UiRect::axes(Val::Px(10.), Val::Px(14.)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.055, 0.058, 0.065)),
                    BorderColor::all(BORDER),
                ))
                .with_children(|sidebar| {
                    sidebar
                        .spawn(Node {
                            width: Val::Percent(100.),
                            height: Val::Px(58.),
                            padding: UiRect::horizontal(Val::Px(12.)),
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(12.),
                            margin: UiRect::bottom(Val::Px(8.)),
                            ..default()
                        })
                        .with_children(|brand| {
                            brand
                                .spawn((
                                    Node {
                                        width: Val::Px(20.),
                                        height: Val::Px(20.),
                                        border: UiRect::all(Val::Px(3.)),
                                        ..default()
                                    },
                                    BorderColor::all(Color::srgb(0.95, 0.05, 0.04)),
                                ))
                                .with_child((
                                    Node {
                                        position_type: PositionType::Absolute,
                                        width: Val::Px(8.),
                                        height: Val::Px(8.),
                                        top: Val::Px(-3.),
                                        left: Val::Px(-3.),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.055, 0.058, 0.065)),
                                ));
                            brand.spawn((
                                Text::new("JX手游助手"),
                                ui_text(font.clone(), 17., TEXT),
                            ));
                        });
                    spawn_nav_item(sidebar, font.clone(), "▣   设备", true, NativePage::Device);
                    spawn_nav_item(sidebar, font.clone(), "⌨   映射", false, NativePage::Mapping);
                    spawn_nav_item(sidebar, font.clone(), "⚙   背景设置", false, NativePage::Settings);
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
                            main.spawn(Node {
                                width: Val::Percent(100.),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(5.),
                                ..default()
                            })
                            .with_children(|heading| {
                                heading.spawn(Node {
                                    align_items: AlignItems::Center,
                                    column_gap: Val::Px(10.),
                                    ..default()
                                })
                                .with_children(|title| {
                                    title.spawn((
                                        Node { width: Val::Px(3.), height: Val::Px(28.), ..default() },
                                        BackgroundColor(ACCENT),
                                    ));
                                    title.spawn((Text::new("USB设备连接"), ui_text(font.clone(), 30., TEXT)));
                                });
                                heading.spawn((
                                    Text::new("USB ADB  ·  单设备模式"),
                                    ui_text(font.clone(), 13., MUTED),
                                    Node { margin: UiRect::left(Val::Px(13.)), ..default() },
                                ));
                            });

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
                                BackgroundColor(Color::srgb(0.065, 0.075, 0.095)),
                                BorderColor::all(Color::srgb(0.16, 0.26, 0.42)),
                            ))
                            .with_children(|info| {
                                info.spawn((
                                    Text::new("●  第一版支持一台 USB ADB 设备"),
                                    ui_text(font.clone(), 17., TEXT),
                                ));
                                info.spawn((
                                    Text::new("    请在手机上开启 USB 调试并授权此电脑，连接后点击刷新设备"),
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
                                justify_content: JustifyContent::FlexStart,
                                row_gap: Val::Px(14.),
                                border: UiRect::all(Val::Px(1.)),
                                border_radius: BorderRadius::all(Val::Px(6.)),
                                ..default()
                            },
                            BackgroundColor(PANEL),
                            BorderColor::all(BORDER),
                        ))
                        .with_children(|preview| {
                            preview.spawn(Node {
                                width: Val::Percent(100.),
                                height: Val::Px(34.),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::SpaceBetween,
                                ..default()
                            })
                            .with_children(|header| {
                                header.spawn((Text::new("设备预览"), ui_text(font.clone(), 19., TEXT)));
                                header.spawn((
                                    Text::new("●  待投屏"),
                                    ui_text(font.clone(), 12., Color::srgb(0.38, 0.82, 0.52)),
                                ));
                            });
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
                            preview.spawn((
                                Node {
                                    width: Val::Percent(100.),
                                    padding: UiRect::all(Val::Px(14.)),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(6.),
                                    border_radius: BorderRadius::all(Val::Px(6.)),
                                    ..default()
                                },
                                BackgroundColor(PANEL_ALT),
                            ))
                            .with_children(|help| {
                                help.spawn((Text::new("连接说明"), ui_text(font.clone(), 14., TEXT)));
                                help.spawn((
                                    Text::new("开启 USB 调试并确认授权\n刷新后即可启动投屏"),
                                    ui_text(font.clone(), 12., MUTED),
                                ));
                            });
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
                NAV_ACTIVE
            } else {
                Color::NONE
            }),
            NativeNavButton(page),
        ))
        .with_child((
            Text::new(label),
            ui_text(font, 16., if active { NAV_ACTIVE_TEXT } else { TEXT }),
            NativeNavLabel(page),
        ));
}

fn spawn_mapping_page(parent: &mut ChildSpawnerCommands, font: Handle<Font>) {
    let active_file = LocalConfig::get().active_mapping_file;
    let mapping = read_mapping_value(&active_file).unwrap_or_else(|_| empty_mapping_value());
    let mappings = mapping
        .get("mappings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (original_width, original_height) = mapping_original_size(&mapping);

    parent
        .spawn((
            Node {
                width: Val::Percent(100.),
                height: Val::Percent(100.),
                padding: UiRect::all(Val::Px(16.)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.),
                display: Display::None,
                ..default()
            },
            NativeMappingPage,
        ))
        .with_children(|page| {
            page.spawn(Node {
                width: Val::Percent(100.),
                height: Val::Px(42.),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|toolbar| {
                toolbar.spawn((
                    Node {
                        width: Val::Px(230.),
                        height: Val::Px(34.),
                        padding: UiRect::horizontal(Val::Px(10.)),
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(1.)),
                        border_radius: BorderRadius::all(Val::Px(5.)),
                        ..default()
                    },
                    BackgroundColor(PANEL_ALT),
                    BorderColor::all(BORDER),
                )).with_child((
                    Text::new(active_file.clone()),
                    ui_text(font.clone(), 13., TEXT),
                    NativeMappingFileText,
                ));
                spawn_mapping_toolbar_button(toolbar, font.clone(), "启用", NativeMappingToolbarAction::Activate, false);
                spawn_mapping_toolbar_button(toolbar, font.clone(), "保存", NativeMappingToolbarAction::Save, true);
                spawn_mapping_toolbar_button(toolbar, font.clone(), "还原", NativeMappingToolbarAction::Restore, false);
                spawn_mapping_toolbar_button(toolbar, font.clone(), "刷新", NativeMappingToolbarAction::Refresh, true);
                spawn_mapping_toolbar_button(toolbar, font.clone(), "辅助范围", NativeMappingToolbarAction::ToggleGuides, false);
                spawn_mapping_toolbar_button(toolbar, font.clone(), "刷新背景", NativeMappingToolbarAction::RefreshBackground, true);
            });

            page.spawn((
                Node {
                    width: Val::Percent(100.),
                    flex_grow: 1.,
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(10.),
                    ..default()
                }
            ))
            .with_children(|body| {
                body.spawn((
                    Node {
                        width: Val::Px(EDITOR_CANVAS_WIDTH),
                        height: Val::Px(EDITOR_CANVAS_HEIGHT),
                        position_type: PositionType::Relative,
                        overflow: Overflow::clip(),
                        border: UiRect::all(Val::Px(1.)),
                        ..default()
                    },
                    BackgroundColor(Color::BLACK),
                    BorderColor::all(BORDER),
                    RelativeCursorPosition::default(),
                    NativeMappingCanvas,
                ))
                .with_children(|canvas| {
                    canvas.spawn((
                        ImageNode::default(),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.),
                            top: Val::Px(0.),
                            width: Val::Percent(100.),
                            height: Val::Percent(100.),
                            ..default()
                        },
                        ZIndex(-1),
                        NativeMappingBackground,
                    ));
                    canvas.spawn((
                        Text::new(format!("编辑区域  {} × {}", original_width as u32, original_height as u32)),
                        ui_text(font.clone(), 12., MUTED),
                        Node {
                            position_type: PositionType::Absolute,
                            top: Val::Px(6.),
                            left: Val::Px(8.),
                            ..default()
                        },
                    ));
                    for (index, item) in mappings.iter().enumerate() {
                        let position = mapping_position(item).unwrap_or(Vec2::new(original_width / 2., original_height / 2.));
                        spawn_editor_mapping_guide(canvas, index, item, original_width, original_height, position);
                        let size = mapping_button_size(item);
                        let scaled_size = (size / original_width * EDITOR_CANVAS_WIDTH).clamp(34., 112.);
                        let left = position.x / original_width * EDITOR_CANVAS_WIDTH - scaled_size / 2.;
                        let top = position.y / original_height * EDITOR_CANVAS_HEIGHT - scaled_size / 2.;
                        canvas.spawn((
                            Button,
                            Node {
                                width: Val::Px(scaled_size),
                                height: Val::Px(scaled_size),
                                position_type: PositionType::Absolute,
                                left: Val::Px(left),
                                top: Val::Px(top),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                border: UiRect::all(Val::Px(1.)),
                                border_radius: BorderRadius::all(Val::Percent(50.)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.08, 0.08, 0.09, 0.72)),
                            BorderColor::all(Color::srgb(0.75, 0.75, 0.78)),
                            NativeMappingNode(index),
                        )).with_child((
                            Text::new(mapping_short_label(item)),
                            ui_text(font.clone(), 11., TEXT),
                            TextLayout::justify(Justify::Center),
                            NativeMappingNodeLabel(index),
                        ));
                    }
                });

                body.spawn((
                    Node {
                        flex_grow: 1.,
                        height: Val::Percent(100.),
                        padding: UiRect::all(Val::Px(12.)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(7.),
                        overflow: Overflow::scroll_y(),
                        scrollbar_width: 6.,
                        border: UiRect::all(Val::Px(1.)),
                        border_radius: BorderRadius::all(Val::Px(6.)),
                        ..default()
                    },
                    BackgroundColor(PANEL),
                    BorderColor::all(BORDER),
                    ScrollPosition::default(),
                    RelativeCursorPosition::default(),
                    NativeMappingInspectorScroll,
                )).with_children(|inspector| {
                    inspector.spawn((Text::new("添加按键"), ui_text(font.clone(), 16., TEXT)));
                    inspector.spawn(Node {
                        width: Val::Percent(100.),
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(5.),
                        row_gap: Val::Px(5.),
                        ..default()
                    }).with_children(|palette| {
                        for (kind, label) in [
                            ("SingleTap", "单击"), ("RepeatTap", "连点"), ("MultipleTap", "多点"),
                            ("Swipe", "滑动"), ("DirectionPad", "方向盘"), ("MouseCastSpell", "万向拖动"),
                            ("PadCastSpell", "轮盘施法"), ("CancelCast", "取消施法"), ("Observation", "观察视角"),
                            ("Fps", "FPS"), ("Fire", "开火"), ("RawInput", "直控"), ("Script", "脚本"),
                        ] {
                            spawn_add_mapping_button(palette, font.clone(), kind, label);
                        }
                    });
                    inspector.spawn((Text::new("所选按键"), ui_text(font.clone(), 16., TEXT)));
                    inspector.spawn((
                        Text::new("未选择"),
                        ui_text(font.clone(), 12., MUTED),
                        NativeMappingInspectorValue,
                    ));
                    spawn_mapping_property_button(inspector, font.clone(), "绑定键盘按键", NativeMappingPropertyAction::CaptureBinding, false);
                    spawn_mapping_property_button(inspector, font.clone(), "绑定鼠标左键", NativeMappingPropertyAction::BindMouseLeft, false);
                    spawn_mapping_property_button(inspector, font.clone(), "脚本 / 高级参数", NativeMappingPropertyAction::AdvancedEdit, false);
                    spawn_property_row(inspector, font.clone(), "按键大小", NativeMappingPropertyAction::SizeDown, NativeMappingPropertyAction::SizeUp);
                    spawn_property_row(inspector, font.clone(), "随机偏移 X", NativeMappingPropertyAction::RandomXDown, NativeMappingPropertyAction::RandomXUp);
                    spawn_property_row(inspector, font.clone(), "随机偏移 Y", NativeMappingPropertyAction::RandomYDown, NativeMappingPropertyAction::RandomYUp);
                    spawn_mapping_property_button(inspector, font.clone(), "切换随机算法", NativeMappingPropertyAction::CycleRandomAlgorithm, false);
                    spawn_mapping_property_button(inspector, font.clone(), "删除所选按键", NativeMappingPropertyAction::Delete, true);
                    inspector.spawn((Text::new("配置管理"), ui_text(font.clone(), 16., TEXT)));
                    inspector.spawn(Node {
                        width: Val::Percent(100.),
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(5.),
                        row_gap: Val::Px(5.),
                        ..default()
                    }).with_children(|manage| {
                        for (label, action) in [
                            ("新建", NativeMappingManageAction::Create),
                            ("复制", NativeMappingManageAction::Duplicate),
                            ("重命名", NativeMappingManageAction::Rename),
                            ("删除", NativeMappingManageAction::Delete),
                            ("导入/导出", NativeMappingManageAction::OpenFolder),
                        ] {
                            spawn_mapping_manage_button(manage, font.clone(), label, action);
                        }
                    });
                });
            });
            page.spawn((
                Text::new("拖动按键修改位置；保存前会执行完整映射和脚本校验。"),
                ui_text(font.clone(), 13., MUTED),
                NativeMappingEditorStatus,
            ));
            page.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(90.),
                    right: Val::Px(90.),
                    top: Val::Px(70.),
                    bottom: Val::Px(70.),
                    padding: UiRect::all(Val::Px(16.)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.),
                    border: UiRect::all(Val::Px(1.)),
                    border_radius: BorderRadius::all(Val::Px(7.)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.055, 0.058, 0.065, 0.99)),
                BorderColor::all(BORDER),
                ZIndex(200),
                NativeAdvancedEditorOverlay,
            )).with_children(|dialog| {
                dialog.spawn((Text::new("脚本 / 高级参数（JSON）"), ui_text(font.clone(), 19., TEXT)));
                dialog.spawn((
                    Text::new("直接编辑所选映射的完整参数。支持 before_script、after_script 和 Script 映射内容；应用时会先解析，最终保存时再次执行完整校验。"),
                    ui_text(font.clone(), 12., MUTED),
                ));
                dialog.spawn((
                    Node {
                        width: Val::Percent(100.),
                        flex_grow: 1.,
                        padding: UiRect::all(Val::Px(10.)),
                        overflow: Overflow::clip(),
                        border: UiRect::all(Val::Px(1.)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.025, 0.027, 0.03)),
                    BorderColor::all(BORDER),
                )).with_child((
                    Text::new(""),
                    ui_text(font.clone(), 12., TEXT),
                    NativeAdvancedEditorText,
                ));
                dialog.spawn(Node {
                    width: Val::Percent(100.),
                    height: Val::Px(36.),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::FlexEnd,
                    column_gap: Val::Px(8.),
                    ..default()
                }).with_children(|buttons| {
                    for (label, apply, primary) in [("取消", false, false), ("应用", true, true)] {
                        buttons.spawn((
                            Button,
                            Node {
                                min_width: Val::Px(78.), height: Val::Px(34.),
                                align_items: AlignItems::Center, justify_content: JustifyContent::Center,
                                border: UiRect::all(Val::Px(1.)), border_radius: BorderRadius::all(Val::Px(5.)),
                                ..default()
                            },
                            BackgroundColor(if primary { ACCENT } else { PANEL_ALT }),
                            BorderColor::all(if primary { ACCENT } else { BORDER }),
                            NativeAdvancedEditorButton(apply),
                        )).with_child((Text::new(label), ui_text(font.clone(), 13., TEXT)));
                    }
                });
            });
        });
}

fn spawn_mapping_toolbar_button(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    label: &str,
    action: NativeMappingToolbarAction,
    primary: bool,
) {
    parent.spawn((
        Button,
        Node {
            min_width: Val::Px(68.),
            height: Val::Px(34.),
            padding: UiRect::horizontal(Val::Px(10.)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::all(Val::Px(1.)),
            border_radius: BorderRadius::all(Val::Px(5.)),
            ..default()
        },
        BackgroundColor(if primary { ACCENT } else { PANEL_ALT }),
        BorderColor::all(if primary { ACCENT } else { BORDER }),
        NativeMappingToolbarButton(action),
    )).with_child((Text::new(label), ui_text(font, 13., TEXT)));
}

fn spawn_add_mapping_button(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    kind: &'static str,
    label: &str,
) {
    parent.spawn((
        Button,
        Node {
            min_width: Val::Px(70.),
            height: Val::Px(30.),
            padding: UiRect::horizontal(Val::Px(7.)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::all(Val::Px(1.)),
            border_radius: BorderRadius::all(Val::Px(4.)),
            ..default()
        },
        BackgroundColor(PANEL_ALT),
        BorderColor::all(BORDER),
        NativeAddMappingButton(kind),
    )).with_child((Text::new(label), ui_text(font, 11., TEXT)));
}

fn spawn_mapping_property_button(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    label: &str,
    action: NativeMappingPropertyAction,
    danger: bool,
) {
    parent.spawn((
        Button,
        Node {
            width: Val::Percent(100.),
            height: Val::Px(30.),
            padding: UiRect::horizontal(Val::Px(8.)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::all(Val::Px(1.)),
            border_radius: BorderRadius::all(Val::Px(4.)),
            ..default()
        },
        BackgroundColor(if danger { Color::srgb(0.34, 0.06, 0.05) } else { PANEL_ALT }),
        BorderColor::all(if danger { ACCENT } else { BORDER }),
        NativeMappingPropertyButton(action),
    )).with_child((Text::new(label), ui_text(font, 11., TEXT)));
}

fn spawn_property_row(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    label: &str,
    down: NativeMappingPropertyAction,
    up: NativeMappingPropertyAction,
) {
    parent.spawn(Node {
        width: Val::Percent(100.),
        height: Val::Px(32.),
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(5.),
        align_items: AlignItems::Center,
        ..default()
    }).with_children(|row| {
        row.spawn((Text::new(label), ui_text(font.clone(), 11., MUTED), Node { flex_grow: 1., ..default() }));
        for (caption, action) in [("−", down), ("+", up)] {
            row.spawn((
                Button,
                Node {
                    width: Val::Px(32.),
                    height: Val::Px(28.),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(Val::Px(1.)),
                    border_radius: BorderRadius::all(Val::Px(4.)),
                    ..default()
                },
                BackgroundColor(PANEL_ALT),
                BorderColor::all(BORDER),
                NativeMappingPropertyButton(action),
            )).with_child((Text::new(caption), ui_text(font.clone(), 15., TEXT)));
        }
    });
}

fn spawn_mapping_manage_button(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    label: &str,
    action: NativeMappingManageAction,
) {
    parent.spawn((
        Button,
        Node {
            min_width: Val::Px(70.),
            height: Val::Px(28.),
            padding: UiRect::horizontal(Val::Px(7.)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::all(Val::Px(1.)),
            border_radius: BorderRadius::all(Val::Px(4.)),
            ..default()
        },
        BackgroundColor(PANEL_ALT),
        BorderColor::all(BORDER),
        NativeMappingManageButton(action),
    )).with_child((Text::new(label), ui_text(font, 11., TEXT)));
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
            page.spawn((
                Text::new("画质、帧率、编码和声音在下次投屏时生效；显示与控制设置立即生效。"),
                ui_text(font.clone(), 14., MUTED),
            ));
            page.spawn((
                Node {
                    width: Val::Percent(100.),
                    flex_grow: 1.,
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(14.),
                    ..default()
                },
            ))
            .with_children(|columns| {
                spawn_settings_column(
                    columns,
                    font.clone(),
                    "投屏",
                    &[
                        (NativeSetting::VideoMaxSize, "分辨率", "限制最长边；原始分辨率画质最高"),
                        (NativeSetting::VideoMaxFps, "FPS", "设备不支持时编码器会自动降低"),
                        (NativeSetting::VideoBitRate, "视频码率", "越高越清晰，同时占用更多 USB 带宽"),
                        (NativeSetting::VideoCodec, "视频编码", "H.264 兼容性最好，H.265/AV1 取决于手机"),
                        (NativeSetting::CaptureOrientation, "投屏方向", "只旋转电脑画面，不改变手机系统方向"),
                        (NativeSetting::AudioEnabled, "音频转发", "Android 11+；将手机声音传到电脑"),
                        (NativeSetting::AudioCodec, "音频编码", "音频转发启用后使用"),
                        (NativeSetting::AudioBitRate, "音频码率", "音频转发启用后使用"),
                    ],
                );
                spawn_settings_column(
                    columns,
                    font.clone(),
                    "显示与控制",
                    &[
                        (NativeSetting::AlwaysOnTop, "窗口置顶", "让投屏窗口保持在其他窗口前面"),
                        (NativeSetting::TitlebarVisible, "显示投屏标题栏", "显示窗口与 Android 真实控制按钮"),
                        (NativeSetting::MappingOpacity, "按键映射透明度", "调整投屏窗口上的按键提示透明度"),
                        (NativeSetting::StayAwake, "保持唤醒", "投屏期间阻止设备自动休眠"),
                        (NativeSetting::ClipboardSync, "剪贴板同步", "允许电脑与手机同步复制的文本"),
                    ],
                );
            });
            page.spawn((
                Text::new("设置会直接保存到 data/config.json，不再依赖网页接口。"),
                ui_text(font.clone(), 13., MUTED),
                NativePageStatus,
            ));
        });
}

fn spawn_settings_column(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    title: &str,
    rows: &[(NativeSetting, &str, &str)],
) {
    parent
        .spawn((
            Node {
                width: Val::Percent(50.),
                height: Val::Percent(100.),
                padding: UiRect::all(Val::Px(14.)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.),
                border: UiRect::all(Val::Px(1.)),
                border_radius: BorderRadius::all(Val::Px(6.)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(BORDER),
        ))
        .with_children(|column| {
            column.spawn((Text::new(title), ui_text(font.clone(), 19., TEXT)));
            for (setting, label, description) in rows.iter().copied() {
                spawn_setting_row(column, font.clone(), setting, label, description);
            }
        });
}

fn spawn_setting_row(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    setting: NativeSetting,
    label: &str,
    description: &str,
) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.),
                min_height: Val::Px(58.),
                padding: UiRect::axes(Val::Px(12.), Val::Px(8.)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                border: UiRect::all(Val::Px(1.)),
                border_radius: BorderRadius::all(Val::Px(5.)),
                ..default()
            },
            BackgroundColor(PANEL_ALT),
            BorderColor::all(BORDER),
        ))
        .with_children(|row| {
            row.spawn(Node {
                flex_grow: 1.,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.),
                ..default()
            })
            .with_children(|copy| {
                copy.spawn((Text::new(label), ui_text(font.clone(), 14., TEXT)));
                copy.spawn((Text::new(description), ui_text(font.clone(), 11., MUTED)));
            });
            row.spawn((
                Button,
                Node {
                    min_width: Val::Px(126.),
                    height: Val::Px(34.),
                    padding: UiRect::horizontal(Val::Px(10.)),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(Val::Px(1.)),
                    border_radius: BorderRadius::all(Val::Px(5.)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.075, 0.078, 0.086)),
                BorderColor::all(BORDER),
                NativeSettingButton(setting),
            ))
            .with_child((
                Text::new(native_setting_value(setting, &LocalConfig::get())),
                ui_text(font, 13., TEXT),
                NativeSettingValue(setting),
            ));
        });
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
    m_tx: Res<ChannelSenderM>,
    channel: Res<NativeUiChannel>,
    runtime: Res<TokioTasksRuntime>,
    mut editor: ResMut<NativeMappingEditorState>,
    mut page: ResMut<NativePage>,
    mut device_page: Query<
        &mut Node,
        (
            With<NativeDevicePage>,
            Without<NativeMappingPage>,
            Without<NativeSettingsPage>,
        ),
    >,
    mut mapping_page: Query<
        &mut Node,
        (
            With<NativeMappingPage>,
            Without<NativeDevicePage>,
            Without<NativeSettingsPage>,
        ),
    >,
    mut settings_page: Query<
        &mut Node,
        (
            With<NativeSettingsPage>,
            Without<NativeDevicePage>,
            Without<NativeMappingPage>,
        ),
    >,
    mut status: Query<&mut Text, With<NativePageStatus>>,
) {
    if let Some((_, button)) = nav.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) {
        *page = button.0;
    }
    if let Some((_, file)) = mapping.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) {
        let file_name = file.0.clone();
        match read_mapping_value(&file_name) {
            Ok(value) => {
                editor.file = file_name.clone();
                editor.current = value.clone();
                editor.original = value;
                editor.selected = None;
                editor.dirty = false;
                editor.needs_rebuild = true;
                editor.status = format!("正在加载并启用映射：{file_name}");
            }
            Err(error) => {
                editor.status = format!("无法打开映射：{error}");
                return;
            }
        }
        for mut text in status.iter_mut() {
            text.0 = format!("正在加载并启用映射：{file_name}");
        }
        let command_tx = m_tx.0.clone();
        let result_tx = channel.tx.clone();
        runtime.spawn_background_task(move |_ctx| async move {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            let result = match command_tx.send((
                MaskCommand::LoadAndActivateMappingConfig { file_name: file_name.clone() },
                reply_tx,
            )) {
                Ok(()) => reply_rx.await.unwrap_or_else(|_| Err("映射服务已关闭".to_string())),
                Err(error) => Err(format!("无法发送映射加载命令：{error}")),
            };
            let _ = result_tx.send(NativeUiResult::MappingActivated { file: file_name, result });
        });
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
    mut editor: ResMut<NativeMappingEditorState>,
    mut images: ResMut<Assets<Image>>,
    mut dashboard: Query<&mut Node, With<NativeDashboardRoot>>,
    mut projection: Query<&mut Node, (With<ProjectionBodyMarker>, Without<NativeDashboardRoot>)>,
    mut page_status: Query<&mut Text, With<NativePageStatus>>,
    mut mapping_background: Query<&mut ImageNode, With<NativeMappingBackground>>,
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
            NativeUiResult::MappingActivated { file, result } => match result {
                Ok(_) => {
                    LocalConfig::set_active_mapping_file(file.clone());
                    state.status = format!("映射已启用：{file}");
                    for mut text in page_status.iter_mut() {
                        text.0 = state.status.clone();
                    }
                }
                Err(error) => {
                    state.status = format!("映射启用失败：{error}");
                    for mut text in page_status.iter_mut() {
                        text.0 = state.status.clone();
                    }
                }
            },
            NativeUiResult::MappingBackground(result) => match result {
                Ok(bytes) => match Image::from_buffer(
                    &bytes,
                    ImageType::Extension("png"),
                    CompressedImageFormats::empty(),
                    true,
                    ImageSampler::linear(),
                    RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
                ) {
                    Ok(image) => {
                        let handle = images.add(image);
                        for mut background in mapping_background.iter_mut() {
                            *background = ImageNode::new(handle.clone());
                        }
                        editor.status = "编辑背景已从手机刷新".to_string();
                    }
                    Err(error) => editor.status = format!("背景图片解码失败：{error}"),
                },
                Err(error) => editor.status = format!("刷新背景失败：{error}"),
            },
        }
    }
}

fn sync_native_dashboard(
    state: Res<NativeDeviceState>,
    mut identities: Query<
        &mut Text,
        (
            With<DeviceIdentityText>,
            Without<DeviceStatusText>,
            Without<FooterStatusText>,
        ),
    >,
    mut statuses: Query<
        &mut Text,
        (
            With<DeviceStatusText>,
            Without<DeviceIdentityText>,
            Without<FooterStatusText>,
        ),
    >,
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
    mut nav: Query<
        (&NativeNavButton, &Interaction, &mut BackgroundColor),
        (Without<MappingFileButton>, Without<NativeSettingButton>),
    >,
    mut controls: Query<
        (&Interaction, &mut BackgroundColor),
        (
            Without<NativeNavButton>,
            Or<(With<MappingFileButton>, With<NativeSettingButton>)>,
        ),
    >,
    mut labels: Query<(&NativeNavLabel, &mut TextColor)>,
) {
    for (button, interaction, mut background) in nav.iter_mut() {
        *background = if button.0 == *page {
            NAV_ACTIVE.into()
        } else {
            match *interaction {
                Interaction::Hovered => PANEL_ALT.into(),
                Interaction::Pressed => Color::srgb(0.2, 0.05, 0.04).into(),
                Interaction::None => Color::NONE.into(),
            }
        };
    }
    for (label, mut color) in labels.iter_mut() {
        color.0 = if label.0 == *page { NAV_ACTIVE_TEXT } else { TEXT };
    }
    for (interaction, mut background) in controls.iter_mut() {
        *background = match *interaction {
            Interaction::Hovered => ACCENT_HOVER.into(),
            Interaction::Pressed => Color::srgb(0.42, 0.06, 0.045).into(),
            Interaction::None => PANEL_ALT.into(),
        };
    }
}

fn native_setting_value(setting: NativeSetting, config: &LocalConfig) -> String {
    match setting {
        NativeSetting::VideoMaxSize => match config.video_max_size {
            0 => "原始分辨率".to_string(),
            value => format!("{value}p"),
        },
        NativeSetting::VideoMaxFps => match config.video_max_fps {
            0 => "不限制".to_string(),
            value => format!("{value} FPS"),
        },
        NativeSetting::VideoBitRate => format!("{} Mbps", config.video_bit_rate / 1_000_000),
        NativeSetting::VideoCodec => config.video_codec.to_string().to_uppercase(),
        NativeSetting::CaptureOrientation => match config.capture_orientation {
            -1 => "跟随手机".to_string(),
            value => format!("{value}°"),
        },
        NativeSetting::AudioEnabled => on_off(config.audio_enabled),
        NativeSetting::AudioCodec => config.audio_codec.to_string().to_uppercase(),
        NativeSetting::AudioBitRate => format!("{} Kbps", config.audio_bit_rate / 1_000),
        NativeSetting::AlwaysOnTop => on_off(config.always_on_top),
        NativeSetting::TitlebarVisible => on_off(config.titlebar_visible),
        NativeSetting::MappingOpacity => format!("{}%", (config.mapping_label_opacity * 100.).round()),
        NativeSetting::StayAwake => on_off(config.stay_awake),
        NativeSetting::ClipboardSync => on_off(config.clipboard_sync),
    }
}

fn on_off(value: bool) -> String {
    if value { "开启".to_string() } else { "关闭".to_string() }
}

fn handle_native_settings(
    buttons: Query<(&Interaction, &NativeSettingButton), Changed<Interaction>>,
    mut window: Single<&mut Window>,
    mut status: Query<&mut Text, With<NativePageStatus>>,
) {
    let Some((_, button)) = buttons
        .iter()
        .find(|(interaction, _)| **interaction == Interaction::Pressed)
    else {
        return;
    };

    let config = LocalConfig::get();
    let message = match button.0 {
        NativeSetting::VideoMaxSize => {
            let next = cycle_u32(config.video_max_size, &[0, 360, 480, 720, 1080]);
            LocalConfig::set_video_max_size(next);
            "分辨率已保存，下次投屏生效"
        }
        NativeSetting::VideoMaxFps => {
            let next = cycle_u32(config.video_max_fps, &[0, 30, 60, 90, 120]);
            LocalConfig::set_video_max_fps(next);
            "FPS 已保存，下次投屏生效"
        }
        NativeSetting::VideoBitRate => {
            let next = cycle_u32(config.video_bit_rate, &[4_000_000, 8_000_000, 10_000_000, 16_000_000, 20_000_000]);
            LocalConfig::set_video_bit_rate(next);
            "视频码率已保存，下次投屏生效"
        }
        NativeSetting::VideoCodec => {
            let next = match config.video_codec {
                VideoCodec::H264 => VideoCodec::H265,
                VideoCodec::H265 => VideoCodec::AV1,
                VideoCodec::AV1 => VideoCodec::H264,
            };
            LocalConfig::set_video_codec(next);
            "视频编码已保存，下次投屏生效"
        }
        NativeSetting::CaptureOrientation => {
            let next = match config.capture_orientation {
                -1 => 0,
                0 => 90,
                90 => 180,
                180 => 270,
                _ => -1,
            };
            LocalConfig::set_capture_orientation(next);
            "投屏方向已保存，下次投屏生效"
        }
        NativeSetting::AudioEnabled => {
            LocalConfig::set_audio_enabled(!config.audio_enabled);
            "音频转发已保存，下次投屏生效"
        }
        NativeSetting::AudioCodec => {
            let next = match config.audio_codec {
                AudioCodec::Opus => AudioCodec::Aac,
                AudioCodec::Aac => AudioCodec::Flac,
                AudioCodec::Flac => AudioCodec::Raw,
                AudioCodec::Raw => AudioCodec::Opus,
            };
            LocalConfig::set_audio_codec(next);
            "音频编码已保存，下次投屏生效"
        }
        NativeSetting::AudioBitRate => {
            let next = cycle_u32(config.audio_bit_rate, &[64_000, 128_000, 256_000]);
            LocalConfig::set_audio_bit_rate(next);
            "音频码率已保存，下次投屏生效"
        }
        NativeSetting::AlwaysOnTop => {
            let next = !config.always_on_top;
            LocalConfig::set_always_on_top(next);
            window.window_level = if next { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal };
            "窗口置顶已立即更新"
        }
        NativeSetting::TitlebarVisible => {
            LocalConfig::set_titlebar_visible(!config.titlebar_visible);
            "标题栏设置已保存，下次投屏生效"
        }
        NativeSetting::MappingOpacity => {
            let current = (config.mapping_label_opacity * 100.).round() as u32;
            let next = cycle_u32(current, &[0, 20, 40, 60, 80, 100]);
            LocalConfig::set_mapping_label_opacity(next as f32 / 100.);
            "按键映射透明度已立即更新"
        }
        NativeSetting::StayAwake => {
            LocalConfig::set_stay_awake(!config.stay_awake);
            "保持唤醒已保存，下次投屏生效"
        }
        NativeSetting::ClipboardSync => {
            LocalConfig::set_clipboard_sync(!config.clipboard_sync);
            "剪贴板同步已保存"
        }
    };

    for mut text in status.iter_mut() {
        text.0 = message.to_string();
    }
}

fn cycle_u32(current: u32, values: &[u32]) -> u32 {
    let index = values.iter().position(|value| *value == current).unwrap_or(0);
    values[(index + 1) % values.len()]
}

fn sync_native_setting_values(
    mut values: Query<(&NativeSettingValue, &mut Text)>,
) {
    let config = LocalConfig::get();
    for (setting, mut text) in values.iter_mut() {
        text.0 = native_setting_value(setting.0, &config);
    }
}

fn handle_native_mapping_toolbar(
    buttons: Query<(&Interaction, &NativeMappingToolbarButton), Changed<Interaction>>,
    mut editor: ResMut<NativeMappingEditorState>,
    m_tx: Res<ChannelSenderM>,
    channel: Res<NativeUiChannel>,
    runtime: Res<TokioTasksRuntime>,
    devices: Res<NativeDeviceState>,
) {
    let Some((_, button)) = buttons.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) else {
        return;
    };
    match button.0 {
        NativeMappingToolbarAction::Save => {
            match validate_and_save_mapping_value(&editor.file, &editor.current) {
                Ok(()) => {
                    editor.original = editor.current.clone();
                    editor.dirty = false;
                    editor.status = format!("已保存并通过校验：{}", editor.file);
                }
                Err(error) => editor.status = format!("保存失败：{error}"),
            }
        }
        NativeMappingToolbarAction::Restore => {
            editor.current = editor.original.clone();
            editor.selected = None;
            editor.dragging = None;
            editor.dirty = false;
            editor.needs_rebuild = true;
            editor.status = "已还原到上次保存状态".to_string();
        }
        NativeMappingToolbarAction::Refresh => match read_mapping_value(&editor.file) {
            Ok(value) => {
                editor.current = value.clone();
                editor.original = value;
                editor.selected = None;
                editor.dragging = None;
                editor.dirty = false;
                editor.needs_rebuild = true;
                editor.status = "已从磁盘重新读取映射和背景尺寸".to_string();
            }
            Err(error) => editor.status = format!("刷新失败：{error}"),
        },
        NativeMappingToolbarAction::ToggleGuides => {
            editor.show_guides = !editor.show_guides;
            editor.status = format!("辅助范围已{}", if editor.show_guides { "显示" } else { "隐藏" });
        }
        NativeMappingToolbarAction::RefreshBackground => {
            let Some(device_id) = devices.devices.first().map(|device| device.id.clone()) else {
                editor.status = "刷新背景失败：没有已授权的 USB ADB 设备".to_string();
                return;
            };
            editor.status = "正在从手机截取编辑背景…".to_string();
            let result_tx = channel.tx.clone();
            std::thread::spawn(move || {
                let _ = result_tx.send(NativeUiResult::MappingBackground(capture_adb_screenshot(&device_id)));
            });
        }
        NativeMappingToolbarAction::Activate => {
            if let Err(error) = validate_mapping_value(&editor.current) {
                editor.status = format!("启用失败：{error}");
                return;
            }
            let file_name = editor.file.clone();
            editor.status = format!("正在启用：{file_name}");
            let command_tx = m_tx.0.clone();
            let result_tx = channel.tx.clone();
            runtime.spawn_background_task(move |_ctx| async move {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let result = match command_tx.send((
                    MaskCommand::LoadAndActivateMappingConfig { file_name: file_name.clone() },
                    reply_tx,
                )) {
                    Ok(()) => reply_rx.await.unwrap_or_else(|_| Err("映射服务已关闭".to_string())),
                    Err(error) => Err(format!("无法发送映射加载命令：{error}")),
                };
                let _ = result_tx.send(NativeUiResult::MappingActivated { file: file_name, result });
            });
        }
    }
}

fn handle_native_mapping_selection_and_drag(
    nodes: Query<(&Interaction, &NativeMappingNode), Changed<Interaction>>,
    canvas: Single<&RelativeCursorPosition, With<NativeMappingCanvas>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut editor: ResMut<NativeMappingEditorState>,
) {
    if let Some((_, node)) = nodes.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) {
        editor.selected = Some(node.0);
        editor.dragging = Some(node.0);
    }
    if mouse.just_released(MouseButton::Left) {
        editor.dragging = None;
    }
    let Some(index) = editor.dragging else { return; };
    if !mouse.pressed(MouseButton::Left) { return; }
    let Some(cursor) = canvas.normalized else { return; };
    let (width, height) = mapping_original_size(&editor.current);
    let position = Vec2::new(
        ((cursor.x + 0.5) * width).clamp(0., width),
        ((cursor.y + 0.5) * height).clamp(0., height),
    );
    if set_mapping_position(&mut editor.current, index, position) {
        editor.dirty = true;
        editor.status = format!("位置：{:.0}, {:.0}（尚未保存）", position.x, position.y);
    }
}

fn handle_native_mapping_properties(
    buttons: Query<(&Interaction, &NativeMappingPropertyButton), Changed<Interaction>>,
    mut editor: ResMut<NativeMappingEditorState>,
) {
    let Some(index) = editor.selected else { return; };
    let Some((_, button)) = buttons.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) else {
        return;
    };
    let Some(mappings) = editor.current.get_mut("mappings").and_then(Value::as_array_mut) else { return; };
    if index >= mappings.len() { return; }
    match button.0 {
        NativeMappingPropertyAction::CaptureBinding => {
            if mappings[index].get("type").and_then(Value::as_str) == Some("DirectionPad") {
                mappings[index]["bind"] = json!({
                    "type": "Button", "up": ["KeyW"], "down": ["KeyS"],
                    "left": ["KeyA"], "right": ["KeyD"]
                });
                editor.dirty = true;
                editor.status = "方向盘已绑定为 WASD（尚未保存）".to_string();
            } else {
                editor.capturing_binding = true;
                editor.status = "请按下要绑定的键盘按键；Esc 取消".to_string();
            }
            return;
        }
        NativeMappingPropertyAction::BindMouseLeft => {
            mappings[index]["bind"] = json!(["M-Left"]);
            editor.dirty = true;
            editor.status = "已绑定鼠标左键（尚未保存）".to_string();
            return;
        }
        NativeMappingPropertyAction::AdvancedEdit => {
            match serde_json::to_string_pretty(&mappings[index]) {
                Ok(text) => {
                    editor.advanced_edit = Some((index, text));
                    editor.status = "正在编辑脚本 / 高级参数".to_string();
                }
                Err(error) => editor.status = format!("无法打开高级编辑器：{error}"),
            }
            return;
        }
        NativeMappingPropertyAction::Delete => {
            mappings.remove(index);
            editor.selected = None;
            editor.dragging = None;
            editor.needs_rebuild = true;
            editor.dirty = true;
            editor.status = "已删除按键（尚未保存）".to_string();
            return;
        }
        NativeMappingPropertyAction::SizeDown => adjust_number(&mut mappings[index], "button_size", -8., 24., 320.),
        NativeMappingPropertyAction::SizeUp => adjust_number(&mut mappings[index], "button_size", 8., 24., 320.),
        NativeMappingPropertyAction::RandomXDown => adjust_number(&mut mappings[index], "random_offset_x", -1., 0., 200.),
        NativeMappingPropertyAction::RandomXUp => adjust_number(&mut mappings[index], "random_offset_x", 1., 0., 200.),
        NativeMappingPropertyAction::RandomYDown => adjust_number(&mut mappings[index], "random_offset_y", -1., 0., 200.),
        NativeMappingPropertyAction::RandomYUp => adjust_number(&mut mappings[index], "random_offset_y", 1., 0., 200.),
        NativeMappingPropertyAction::CycleRandomAlgorithm => cycle_random_algorithm(&mut mappings[index]),
    }
    editor.dirty = true;
    editor.status = "按键参数已修改（尚未保存）".to_string();
}

fn capture_native_mapping_binding(
    mut events: MessageReader<KeyboardInput>,
    mut editor: ResMut<NativeMappingEditorState>,
) {
    if !editor.capturing_binding {
        return;
    }
    let Some(event) = events.read().find(|event| event.state == ButtonState::Pressed) else {
        return;
    };
    if event.key_code == KeyCode::Escape {
        editor.capturing_binding = false;
        editor.status = "已取消按键绑定".to_string();
        return;
    }
    let Some(index) = editor.selected else {
        editor.capturing_binding = false;
        return;
    };
    let key = MergedButton::from(event.key_code).to_string();
    let Some(mapping) = editor.current.get_mut("mappings").and_then(Value::as_array_mut).and_then(|items| items.get_mut(index)) else {
        editor.capturing_binding = false;
        return;
    };
    mapping["bind"] = json!([key.clone()]);
    editor.capturing_binding = false;
    editor.dirty = true;
    editor.status = format!("已绑定 {key}（尚未保存）");
}

fn handle_native_advanced_editor(
    mut events: MessageReader<KeyboardInput>,
    buttons: Query<(&Interaction, &NativeAdvancedEditorButton), Changed<Interaction>>,
    mut editor: ResMut<NativeMappingEditorState>,
) {
    if let Some((_, button)) = buttons.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) {
        if !button.0 {
            editor.advanced_edit = None;
            editor.status = "已取消高级参数编辑".to_string();
            return;
        }
        let Some((index, buffer)) = editor.advanced_edit.clone() else { return; };
        match serde_json::from_str::<Value>(&buffer) {
            Ok(value) => {
                let Some(mappings) = editor.current.get_mut("mappings").and_then(Value::as_array_mut) else { return; };
                if index < mappings.len() {
                    mappings[index] = value;
                    match validate_mapping_value(&editor.current) {
                        Ok(()) => {
                            editor.advanced_edit = None;
                            editor.dirty = true;
                            editor.needs_rebuild = true;
                            editor.status = "高级参数已应用（尚未保存）".to_string();
                        }
                        Err(error) => editor.status = format!("参数校验失败：{error}"),
                    }
                }
            }
            Err(error) => editor.status = format!("JSON 格式错误：{error}"),
        }
        return;
    }

    let mut cancelled = false;
    if let Some((_, buffer)) = editor.advanced_edit.as_mut() {
        for event in events.read().filter(|event| event.state == ButtonState::Pressed) {
            match event.key_code {
                KeyCode::Escape => {
                    cancelled = true;
                    break;
                }
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Enter | KeyCode::NumpadEnter => buffer.push('\n'),
                KeyCode::Tab => buffer.push_str("  "),
                _ => {
                    if let Some(text) = event.text.as_ref() {
                        buffer.push_str(text);
                    }
                }
            }
        }
    }
    if cancelled {
        editor.advanced_edit = None;
        editor.status = "已取消高级参数编辑".to_string();
    }
}

fn sync_native_advanced_editor(
    editor: Res<NativeMappingEditorState>,
    mut overlay: Query<&mut Node, With<NativeAdvancedEditorOverlay>>,
    mut text: Query<&mut Text, With<NativeAdvancedEditorText>>,
) {
    let visible = editor.advanced_edit.is_some();
    for mut node in overlay.iter_mut() {
        node.display = if visible { Display::Flex } else { Display::None };
    }
    let content = editor.advanced_edit.as_ref().map(|(_, value)| value.as_str()).unwrap_or("");
    for mut value in text.iter_mut() {
        value.0 = content.to_string();
    }
}

fn handle_native_add_mapping(
    buttons: Query<(&Interaction, &NativeAddMappingButton), Changed<Interaction>>,
    mut editor: ResMut<NativeMappingEditorState>,
) {
    let Some((_, button)) = buttons.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) else {
        return;
    };
    let (width, height) = mapping_original_size(&editor.current);
    let pointer_id = next_pointer_id(&editor.current);
    let mapping = new_mapping_value(button.0, Vec2::new(width / 2., height / 2.), pointer_id);
    let Some(mappings) = editor.current.get_mut("mappings").and_then(Value::as_array_mut) else { return; };
    mappings.push(mapping);
    editor.selected = Some(mappings.len() - 1);
    editor.dirty = true;
    editor.needs_rebuild = true;
    editor.status = format!("已添加 {}（尚未保存）", button.0);
}

fn handle_native_mapping_management(
    buttons: Query<(&Interaction, &NativeMappingManageButton), Changed<Interaction>>,
    mut editor: ResMut<NativeMappingEditorState>,
) {
    let Some((_, button)) = buttons.iter().find(|(interaction, _)| **interaction == Interaction::Pressed) else {
        return;
    };
    let mapping_dir = relate_to_data_path(["mapping"]);
    let result = match button.0 {
        NativeMappingManageAction::Create => {
            let file = unique_mapping_file("new-mapping", &mapping_dir);
            let value = empty_mapping_value();
            validate_and_save_mapping_value(&file, &value).map(|_| (file, value))
        }
        NativeMappingManageAction::Duplicate => {
            let stem = editor.file.trim_end_matches(".json");
            let file = unique_mapping_file(&format!("{stem}-copy"), &mapping_dir);
            validate_and_save_mapping_value(&file, &editor.current).map(|_| (file, editor.current.clone()))
        }
        NativeMappingManageAction::Rename => {
            let stem = editor.file.trim_end_matches(".json");
            let file = unique_mapping_file(&format!("{stem}-renamed"), &mapping_dir);
            let old_path = mapping_dir.join(&editor.file);
            let new_path = mapping_dir.join(&file);
            fs::rename(&old_path, &new_path)
                .map_err(|error| format!("重命名失败：{error}"))
                .map(|_| {
                    if LocalConfig::get().active_mapping_file == editor.file {
                        LocalConfig::set_active_mapping_file(file.clone());
                    }
                    (file, editor.current.clone())
                })
        }
        NativeMappingManageAction::Delete => {
            if LocalConfig::get().active_mapping_file == editor.file {
                Err("当前启用的映射不能删除，请先启用其他配置".to_string())
            } else {
                match fs::remove_file(mapping_dir.join(&editor.file)) {
                    Err(error) => Err(format!("删除失败：{error}")),
                    Ok(()) => match first_mapping_file(&mapping_dir) {
                        Some(file) => read_mapping_value(&file).map(|value| (file, value)),
                        None => Err("没有可继续编辑的映射配置".to_string()),
                    },
                }
            }
        }
        NativeMappingManageAction::OpenFolder => {
            #[cfg(target_os = "windows")]
            let opened = std::process::Command::new("explorer").arg(&mapping_dir).spawn();
            #[cfg(not(target_os = "windows"))]
            let opened = std::process::Command::new("xdg-open").arg(&mapping_dir).spawn();
            match opened {
                Ok(_) => {
                    editor.status = "已打开映射目录，可直接导入或导出 JSON 文件".to_string();
                    return;
                }
                Err(error) => Err(format!("无法打开映射目录：{error}")),
            }
        }
    };
    match result {
        Ok((file, value)) => {
            editor.file = file.clone();
            editor.current = value.clone();
            editor.original = value;
            editor.selected = None;
            editor.dragging = None;
            editor.dirty = false;
            editor.needs_rebuild = true;
            editor.status = format!("正在编辑：{file}");
        }
        Err(error) => editor.status = error,
    }
}

fn rebuild_native_mapping_nodes(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut editor: ResMut<NativeMappingEditorState>,
    canvas: Single<Entity, With<NativeMappingCanvas>>,
    nodes: Query<Entity, With<NativeMappingNode>>,
    guides: Query<Entity, With<NativeMappingGuide>>,
) {
    if !editor.needs_rebuild { return; }
    for entity in nodes.iter() {
        commands.entity(entity).despawn();
    }
    for entity in guides.iter() {
        commands.entity(entity).despawn();
    }
    let font = asset_server.load("fonts/NotoSansSC-Regular.otf");
    let (width, height) = mapping_original_size(&editor.current);
    let mappings = editor.current.get("mappings").and_then(Value::as_array).cloned().unwrap_or_default();
    commands.entity(*canvas).with_children(|parent| {
        for (index, item) in mappings.iter().enumerate() {
            let position = mapping_position(item).unwrap_or(Vec2::new(width / 2., height / 2.));
            spawn_editor_mapping_guide(parent, index, item, width, height, position);
            spawn_editor_mapping_node(parent, font.clone(), index, item, width, height);
        }
    });
    editor.needs_rebuild = false;
}

fn sync_native_mapping_editor(
    editor: Res<NativeMappingEditorState>,
    mut nodes: Query<(&NativeMappingNode, &mut Node, &mut BackgroundColor)>,
    mut guides: Query<(&NativeMappingGuide, &mut Node), Without<NativeMappingNode>>,
    mut status: Query<
        &mut Text,
        (With<NativeMappingEditorStatus>, Without<NativeMappingInspectorValue>, Without<NativeMappingFileText>),
    >,
    mut inspector: Query<
        &mut Text,
        (With<NativeMappingInspectorValue>, Without<NativeMappingEditorStatus>, Without<NativeMappingFileText>),
    >,
    mut file_text: Query<
        &mut Text,
        (With<NativeMappingFileText>, Without<NativeMappingEditorStatus>, Without<NativeMappingInspectorValue>),
    >,
) {
    let (width, height) = mapping_original_size(&editor.current);
    let mappings = editor.current.get("mappings").and_then(Value::as_array);
    for (node, mut style, mut background) in nodes.iter_mut() {
        let Some(item) = mappings.and_then(|items| items.get(node.0)) else { continue; };
        let Some(position) = mapping_position(item) else { continue; };
        let size = (mapping_button_size(item) / width * EDITOR_CANVAS_WIDTH).clamp(34., 112.);
        style.width = Val::Px(size);
        style.height = Val::Px(size);
        style.left = Val::Px(position.x / width * EDITOR_CANVAS_WIDTH - size / 2.);
        style.top = Val::Px(position.y / height * EDITOR_CANVAS_HEIGHT - size / 2.);
        *background = if editor.selected == Some(node.0) {
            Color::srgba(0.78, 0.12, 0.09, 0.82).into()
        } else {
            Color::srgba(0.08, 0.08, 0.09, 0.72).into()
        };
    }
    for (guide, mut style) in guides.iter_mut() {
        let Some(item) = mappings.and_then(|items| items.get(guide.0)) else { continue; };
        let Some(position) = mapping_position(item) else { continue; };
        let radius = mapping_guide_radius(item);
        let radius_x = radius / width * EDITOR_CANVAS_WIDTH;
        let radius_y = radius / height * EDITOR_CANVAS_HEIGHT;
        style.display = if editor.show_guides && radius > 0. { Display::Flex } else { Display::None };
        style.width = Val::Px(radius_x * 2.);
        style.height = Val::Px(radius_y * 2.);
        style.left = Val::Px(position.x / width * EDITOR_CANVAS_WIDTH - radius_x);
        style.top = Val::Px(position.y / height * EDITOR_CANVAS_HEIGHT - radius_y);
    }
    for mut text in status.iter_mut() {
        text.0 = editor.status.clone();
    }
    let details = editor.selected
        .and_then(|index| mappings.and_then(|items| items.get(index)))
        .map(mapping_inspector_text)
        .unwrap_or_else(|| "未选择".to_string());
    for mut text in inspector.iter_mut() {
        text.0 = details.clone();
    }
    for mut text in file_text.iter_mut() {
        text.0 = if editor.dirty { format!("{}  *", editor.file) } else { editor.file.clone() };
    }
}

fn sync_native_mapping_node_labels(
    editor: Res<NativeMappingEditorState>,
    mut labels: Query<(&NativeMappingNodeLabel, &mut Text)>,
) {
    let mappings = editor.current.get("mappings").and_then(Value::as_array);
    for (label, mut text) in labels.iter_mut() {
        if let Some(mapping) = mappings.and_then(|items| items.get(label.0)) {
            text.0 = mapping_short_label(mapping);
        }
    }
}

fn scroll_native_mapping_inspector(
    mut wheel: MessageReader<MouseWheel>,
    inspector: Single<
        (&mut ScrollPosition, &ComputedNode, &RelativeCursorPosition),
        With<NativeMappingInspectorScroll>,
    >,
) {
    let (mut scroll_position, computed, cursor) = inspector.into_inner();
    if !cursor.cursor_over() {
        return;
    }
    let visible_height = computed.size().y;
    let content_height = computed.content_size().y;
    let max_scroll = ((content_height - visible_height).max(0.)) * computed.inverse_scale_factor();
    for event in wheel.read() {
        let delta = match event.unit {
            MouseScrollUnit::Line => event.y * 24.,
            MouseScrollUnit::Pixel => event.y,
        };
        scroll_position.0.y = (scroll_position.0.y - delta).clamp(0., max_scroll);
    }
}

fn empty_mapping_value() -> Value {
    json!({
        "version": "1",
        "original_size": { "width": 1920, "height": 1080 },
        "mappings": []
    })
}

fn read_mapping_value(file: &str) -> Result<Value, String> {
    let path = relate_to_data_path(["mapping", file]);
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|error| format!("JSON 格式错误：{error}"))?;
    validate_mapping_value(&value)?;
    Ok(value)
}

fn validate_mapping_value(value: &Value) -> Result<(), String> {
    let config: MappingConfig = serde_json::from_value(value.clone())
        .map_err(|error| format!("映射结构错误：{error}"))?;
    let diagnostics = validate_mapping_config_diagnostics(&config);
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics
            .iter()
            .map(|item| item.message.as_str())
            .collect::<Vec<_>>()
            .join("；"))
    }
}

fn validate_and_save_mapping_value(file: &str, value: &Value) -> Result<(), String> {
    validate_mapping_value(value)?;
    let path = relate_to_data_path(["mapping", file]);
    let content = serde_json::to_string_pretty(value)
        .map_err(|error| format!("无法序列化映射：{error}"))?;
    fs::write(&path, content).map_err(|error| format!("无法写入 {}：{error}", path.display()))
}

fn unique_mapping_file(prefix: &str, directory: &std::path::Path) -> String {
    for index in 1..=9999 {
        let candidate = format!("{prefix}-{index}.json");
        if !directory.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{prefix}-{:08x}.json", rand::random::<u32>())
}

fn first_mapping_file(directory: &std::path::Path) -> Option<String> {
    let mut files = fs::read_dir(directory).ok()?.flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                path.file_name().map(|name| name.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    files.sort();
    files.into_iter().next()
}

fn mapping_original_size(value: &Value) -> (f32, f32) {
    let width = value.pointer("/original_size/width").and_then(Value::as_f64).unwrap_or(1920.) as f32;
    let height = value.pointer("/original_size/height").and_then(Value::as_f64).unwrap_or(1080.) as f32;
    (width.max(1.), height.max(1.))
}

fn mapping_position(value: &Value) -> Option<Vec2> {
    let position = match value.get("type").and_then(Value::as_str) {
        Some("MultipleTap") => value.pointer("/items/0/position"),
        Some("Swipe") => value.pointer("/positions/0"),
        _ => value.get("position"),
    }?;
    Some(Vec2::new(
        position.get("x")?.as_f64()? as f32,
        position.get("y")?.as_f64()? as f32,
    ))
}

fn set_mapping_position(config: &mut Value, index: usize, position: Vec2) -> bool {
    let Some(mapping) = config.get_mut("mappings").and_then(Value::as_array_mut).and_then(|items| items.get_mut(index)) else {
        return false;
    };
    let kind = mapping.get("type").and_then(Value::as_str).unwrap_or_default();
    let target = match kind {
        "MultipleTap" => mapping.pointer_mut("/items/0/position"),
        "Swipe" => mapping.pointer_mut("/positions/0"),
        _ => mapping.get_mut("position"),
    };
    let Some(target) = target else { return false; };
    *target = json!({ "x": position.x, "y": position.y });
    true
}

fn mapping_button_size(value: &Value) -> f32 {
    value.get("button_size").and_then(Value::as_f64).unwrap_or(64.) as f32
}

fn mapping_guide_radius(value: &Value) -> f32 {
    let number = |field: &str| value.get(field).and_then(Value::as_f64).unwrap_or(0.) as f32;
    match value.get("type").and_then(Value::as_str).unwrap_or_default() {
        "MouseCastSpell" => number("cast_radius").max(number("drag_radius")),
        "PadCastSpell" => number("drag_radius"),
        "DirectionPad" => number("max_offset_x").max(number("max_offset_y")),
        "Observation" => number("max_radius"),
        "Fps" => number("max_offset_x").max(number("max_offset_y")),
        _ => number("random_offset_x").max(number("random_offset_y")),
    }
}

fn mapping_short_label(value: &Value) -> String {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("按键");
    let label = match kind {
        "SingleTap" => "单击", "RepeatTap" => "连点", "MultipleTap" => "多点",
        "Swipe" => "滑动", "DirectionPad" => "方向盘", "MouseCastSpell" => "万向拖动",
        "PadCastSpell" => "轮盘施法", "CancelCast" => "取消施法", "Observation" => "观察",
        "Fps" => "FPS", "Fire" => "开火", "RawInput" => "直控", "Script" => "脚本",
        _ => kind,
    };
    let bind = value.get("bind").and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("+"))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "未绑定".to_string());
    format!("{label}\n{bind}")
}

fn mapping_inspector_text(value: &Value) -> String {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("未知");
    let position = mapping_position(value).unwrap_or(Vec2::ZERO);
    let size = mapping_button_size(value);
    let random_x = value.get("random_offset_x").and_then(Value::as_f64).unwrap_or(0.);
    let random_y = value.get("random_offset_y").and_then(Value::as_f64).unwrap_or(0.);
    let algorithm = value.get("random_offset_algorithm").and_then(Value::as_str).unwrap_or("Bezier");
    format!("{kind}\n位置 {:.0}, {:.0}  |  大小 {:.0}\n随机偏移 X {:.0} / Y {:.0}  |  {algorithm}", position.x, position.y, size, random_x, random_y)
}

fn adjust_number(value: &mut Value, field: &str, delta: f64, min: f64, max: f64) {
    let current = value.get(field).and_then(Value::as_f64).unwrap_or(if field == "button_size" { 64. } else { 0. });
    if let Some(object) = value.as_object_mut() {
        object.insert(field.to_string(), json!((current + delta).clamp(min, max)));
    }
}

fn cycle_random_algorithm(value: &mut Value) {
    const ALGORITHMS: [&str; 5] = ["ExtremeRandom", "Bezier", "Linear", "Sine", "RandomWalk"];
    let current = value.get("random_offset_algorithm").and_then(Value::as_str).unwrap_or("Bezier");
    let index = ALGORITHMS.iter().position(|item| *item == current).unwrap_or(0);
    if let Some(object) = value.as_object_mut() {
        object.insert("random_offset_algorithm".to_string(), json!(ALGORITHMS[(index + 1) % ALGORITHMS.len()]));
    }
}

fn next_pointer_id(config: &Value) -> u32 {
    let used = config.get("mappings").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|item| item.get("pointer_id").and_then(Value::as_u64).map(|value| value as u32))
        .collect::<std::collections::HashSet<_>>();
    (1..=31).find(|id| !used.contains(id)).unwrap_or(31)
}

fn new_mapping_value(kind: &str, position: Vec2, pointer_id: u32) -> Value {
    let id = format!("{:08x}", rand::random::<u32>());
    let pos = json!({ "x": position.x, "y": position.y });
    let hooks = json!({ "before_script": "", "after_script": "" });
    let mut value = match kind {
        "SingleTap" => json!({ "id": id, "bind": [], "duration": 50, "note": "", "pointer_id": pointer_id, "position": pos, "random_offset_x": 10, "random_offset_y": 10, "script_hooks": hooks, "sync": false, "type": kind }),
        "RepeatTap" => json!({ "id": id, "bind": [], "duration": 50, "interval": 100, "note": "", "pointer_id": pointer_id, "position": pos, "random_offset_x": 10, "random_offset_y": 10, "script_hooks": hooks, "type": kind }),
        "MultipleTap" => json!({ "id": id, "bind": [], "items": [{ "duration": 50, "position": pos, "wait": 0 }], "note": "", "pointer_id": pointer_id, "random_offset_x": 10, "random_offset_y": 10, "script_hooks": hooks, "type": kind }),
        "Swipe" => json!({ "id": id, "bind": [], "enable_randomization": false, "duration": 100, "note": "", "pointer_id": pointer_id, "positions": [pos], "script_hooks": hooks, "type": kind }),
        "DirectionPad" => json!({ "id": id, "bind": { "type": "Button", "up": [], "down": [], "left": [], "right": [] }, "button_size": 128, "enable_randomization": false, "initial_duration": 0, "max_offset_x": 200, "max_offset_y": 200, "note": "", "pointer_id": pointer_id, "position": pos, "random_distance_max_scale": 1.1, "random_distance_min_scale": 0.9, "random_offset_x": 10, "random_offset_y": 10, "random_offset_algorithm": "Bezier", "jitter_offset_x": 5, "jitter_offset_y": 5, "script_hooks": hooks, "type": kind, "up_boost_key": null, "up_boost_scale": 2.0 }),
        "MouseCastSpell" => json!({ "id": id, "bind": [], "button_size": 64, "cast_no_direction": false, "cast_radius": 200, "center": pos, "drag_radius": 150, "enable_initial_swipe_randomization": false, "horizontal_scale_factor": 7, "initial_duration": 0, "note": "", "pointer_id": pointer_id, "position": pos, "random_offset_x": 10, "random_offset_y": 10, "random_offset_algorithm": "Bezier", "release_mode": "OnRelease", "script_hooks": hooks, "type": kind, "vertical_scale_factor": 10 }),
        "PadCastSpell" => json!({ "id": id, "bind": [], "block_direction_pad": false, "drag_radius": 150, "enable_randomization": false, "note": "", "pad_bind": { "type": "Button", "up": [], "down": [], "left": [], "right": [] }, "pointer_id": pointer_id, "position": pos, "random_offset_x": 10, "random_offset_y": 10, "release_mode": "OnRelease", "script_hooks": hooks, "type": kind }),
        "CancelCast" => json!({ "id": id, "bind": [], "note": "", "position": pos, "script_hooks": hooks, "type": kind }),
        "Observation" => json!({ "id": id, "bind": [], "max_radius": 0, "note": "", "pointer_id": pointer_id, "position": pos, "random_offset_x": 10, "random_offset_y": 10, "sensitivity_x": 0.8, "sensitivity_y": 0.8, "script_hooks": hooks, "type": kind }),
        "Fps" => json!({ "id": id, "bind": [], "button_size": 64, "note": "", "pointer_id": 0, "position": pos, "random_offset_x": 10, "random_offset_y": 10, "random_offset_algorithm": "Bezier", "sensitivity_x": 0.8, "sensitivity_y": 0.8, "max_offset_x": 0, "max_offset_y": 0, "touch_mode": { "type": "single", "interval": 0 }, "type": kind }),
        "Fire" => json!({ "id": id, "bind": [], "button_size": 64, "note": "", "pointer_id": 0, "position": pos, "preserve_fps_control": true, "random_offset_x": 10, "random_offset_y": 10, "random_offset_algorithm": "Bezier", "sensitivity_x": 0.8, "sensitivity_y": 0.8, "script_hooks": hooks, "type": kind }),
        "RawInput" => json!({ "id": id, "bind": [], "note": "", "position": pos, "type": kind }),
        "Script" => json!({ "id": id, "bind": [], "note": "", "position": pos, "pressed_script": "", "released_script": "", "held_script": "", "interval": 300, "type": kind }),
        _ => json!({ "id": id, "bind": [], "note": "", "position": pos, "type": "SingleTap", "duration": 50, "pointer_id": pointer_id, "random_offset_x": 10, "random_offset_y": 10, "script_hooks": hooks, "sync": false }),
    };
    if value.get("button_size").is_none() {
        value.as_object_mut().unwrap().insert("button_size".to_string(), json!(64));
    }
    value
}

fn spawn_editor_mapping_node(
    parent: &mut ChildSpawnerCommands,
    font: Handle<Font>,
    index: usize,
    item: &Value,
    width: f32,
    height: f32,
) {
    let position = mapping_position(item).unwrap_or(Vec2::new(width / 2., height / 2.));
    let size = (mapping_button_size(item) / width * EDITOR_CANVAS_WIDTH).clamp(34., 112.);
    parent.spawn((
        Button,
        Node {
            width: Val::Px(size), height: Val::Px(size), position_type: PositionType::Absolute,
            left: Val::Px(position.x / width * EDITOR_CANVAS_WIDTH - size / 2.),
            top: Val::Px(position.y / height * EDITOR_CANVAS_HEIGHT - size / 2.),
            align_items: AlignItems::Center, justify_content: JustifyContent::Center,
            border: UiRect::all(Val::Px(1.)), border_radius: BorderRadius::all(Val::Percent(50.)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.08, 0.08, 0.09, 0.72)),
        BorderColor::all(Color::srgb(0.75, 0.75, 0.78)),
        NativeMappingNode(index),
    )).with_child((
        Text::new(mapping_short_label(item)),
        ui_text(font, 11., TEXT),
        TextLayout::justify(Justify::Center),
        NativeMappingNodeLabel(index),
    ));
}

fn spawn_editor_mapping_guide(
    parent: &mut ChildSpawnerCommands,
    index: usize,
    item: &Value,
    width: f32,
    height: f32,
    position: Vec2,
) {
    let radius = mapping_guide_radius(item);
    let radius_x = radius / width * EDITOR_CANVAS_WIDTH;
    let radius_y = radius / height * EDITOR_CANVAS_HEIGHT;
    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
    let (border_color, fill_color) = if matches!(kind, "MouseCastSpell" | "PadCastSpell") {
        (Color::srgba(0.66, 0.33, 0.97, 0.82), Color::srgba(0.66, 0.33, 0.97, 0.08))
    } else {
        (Color::srgba(0.13, 0.78, 0.39, 0.82), Color::srgba(0.13, 0.78, 0.39, 0.08))
    };
    parent.spawn((
        Node {
            width: Val::Px(radius_x * 2.),
            height: Val::Px(radius_y * 2.),
            position_type: PositionType::Absolute,
            left: Val::Px(position.x / width * EDITOR_CANVAS_WIDTH - radius_x),
            top: Val::Px(position.y / height * EDITOR_CANVAS_HEIGHT - radius_y),
            display: if radius > 0. { Display::Flex } else { Display::None },
            border: UiRect::all(Val::Px(1.)),
            border_radius: BorderRadius::all(Val::Percent(50.)),
            ..default()
        },
        BackgroundColor(fill_color),
        BorderColor::all(border_color),
        NativeMappingGuide(index),
    ));
}
