use std::time::Duration;

use bevy::{
    math::CompassOctant,
    prelude::IntoScheduleConfigs,
    prelude::*,
    window::{CursorIcon, PrimaryWindow, SystemCursorIcon, WindowLevel},
    winit::{UpdateMode, WinitSettings},
};
use bevy_ui_render::prelude::MaterialNode;

use crate::{
    config::LocalConfig,
    mask::{
        MaskFrameSet, MaskResizeState,
        mask_command::TitlebarState,
        video::{VideoPlayer, YuvVideoMaterial, create_initial_yuv_material},
    },
    scrcpy::{constant::Keycode, controller::ControllerCommand, device_action},
    utils::{ChannelSenderCS, ChannelSenderD, share::ControlledDevice},
};

pub const BORDER_THICKNESS: f32 = 1.0;
pub const TITLEBAR_HEIGHT: f32 = 30.0;

const RESIZE_HANDLE_SIZE: f32 = 5.0;
const TITLEBAR_TITLE_MIN_WINDOW_WIDTH: f32 = 360.0;

#[derive(Component)]
struct ResizeHandle(CompassOctant);

#[derive(Component)]
pub struct MaskContentMarker;

#[derive(Component)]
pub struct TitlebarMarker;

#[derive(Component)]
pub struct ProjectionBodyMarker;

#[derive(Component)]
struct TitlebarTitleMarker;

#[derive(Component)]
struct MinimizeButton;

#[derive(Component)]
struct MaximizeButton;

#[derive(Component)]
struct CloseButton;

#[derive(Component)]
struct PushpinButton;

#[derive(Component)]
struct TooltipText;

#[derive(Component)]
pub struct DeviceButton(pub DeviceAction);

#[derive(Clone, Copy)]
pub enum DeviceAction {
    Back,
    Home,
    AppSwitch,
    ScreenOff,
    ScreenOn,
    VolumeUp,
    VolumeDown,
}

#[derive(Resource)]
pub struct MaskContentEntity(pub Entity);

pub struct BasicPlugin;

impl Plugin for BasicPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::NONE))
            .insert_resource(WinitSettings {
                focused_mode: UpdateMode::Continuous,
                unfocused_mode: UpdateMode::reactive_low_power(Duration::from_millis(100)),
            })
            .add_systems(Startup, setup_ui)
            .add_systems(
                Update,
                (
                    button_interaction,
                    handle_titlebar_buttons,
                    handle_device_buttons,
                    handle_titlebar_drag,
                    handle_resize.in_set(MaskFrameSet::Resize),
                    sync_resize_cursor,
                    sync_titlebar_visibility,
                    sync_titlebar_title_visibility,
                    sync_pushpin_style,
                    sync_tooltips,
                ),
            );
    }
}

fn setup_ui(
    mut commands: Commands,
    window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut yuv_materials: ResMut<Assets<YuvVideoMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let (window_entity, mut window) = window.into_inner();
    let config = LocalConfig::get();
    let legacy_titlebar_visible = if cfg!(target_os = "windows") {
        false
    } else {
        config.titlebar_visible
    };
    let win_h = if legacy_titlebar_visible {
        600. + TITLEBAR_HEIGHT
    } else {
        600.
    };
    window.resolution.set(800., win_h);
    commands
        .entity(window_entity)
        .insert(CursorIcon::from(SystemCursorIcon::Default));

    commands.spawn(Camera2d::default());

    let titlebar_bg = Color::srgba(0.12, 0.13, 0.15, 0.98);
    let border_color = Color::srgba_u8(183, 42, 32, 255);
    let video_material = create_initial_yuv_material(&mut images, &mut yuv_materials);

    let minimize_icon: Handle<Image> = asset_server.load("icons/minus.png");
    let maximize_icon: Handle<Image> = asset_server.load("icons/border.png");
    let pushpin_icon: Handle<Image> = asset_server.load("icons/pushpin.png");
    let close_icon: Handle<Image> = asset_server.load("icons/close.png");
    let screen_off_icon: Handle<Image> = asset_server.load("icons/bulb.png");
    let screen_on_icon: Handle<Image> = asset_server.load("icons/bulb-fill.png");
    let volume_up_icon: Handle<Image> = asset_server.load("icons/up.png");
    let volume_down_icon: Handle<Image> = asset_server.load("icons/down.png");
    let back_icon: Handle<Image> = asset_server.load("icons/enter.png");
    let home_icon: Handle<Image> = asset_server.load("icons/border.png");
    let menu_icon: Handle<Image> = asset_server.load("icons/menu.png");
    let ui_font: Handle<Font> = asset_server.load("fonts/NotoSansSC-Regular.otf");

    let initial_pin_bg = if config.always_on_top { PIN_ACTIVE_BG } else { NORMAL_BG };

    // Root container
    let root_entity = commands
        .spawn(Node {
            width: Val::Percent(100.),
            height: Val::Percent(100.),
            flex_direction: FlexDirection::Column,
            ..default()
        })
        .id();

    // Titlebar
    let titlebar_entity = commands
        .spawn((
            Node {
                width: Val::Percent(100.),
                height: Val::Px(TITLEBAR_HEIGHT),
                padding: UiRect::px(8., 8., 0., 0.),
                display: if legacy_titlebar_visible {
                    Display::Flex
                } else {
                    Display::None
                },
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(titlebar_bg),
            TitlebarMarker,
            Interaction::default(),
        ))
        .id();

    commands.entity(titlebar_entity).with_children(|titlebar| {
        // Windows-style title: application name on the left.
        titlebar.spawn((
            Text::new("JX手游助手"),
            TextLayout::no_wrap(),
            TextFont {
                font: ui_font.clone().into(),
                font_size: FontSize::Px(14.),
                ..default()
            },
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
            TitlebarTitleMarker,
        ));

        // Spacer
        titlebar.spawn(Node {
            flex_grow: 1.,
            ..default()
        });

        // Windows-style controls on the right: pin, minimize, maximize, close.
        titlebar
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                height: Val::Percent(100.),
                ..default()
            })
            .with_children(|right| {
                for (icon, marker) in [
                    (pushpin_icon, 0_u8),
                    (minimize_icon, 1_u8),
                    (maximize_icon, 2_u8),
                    (close_icon, 3_u8),
                ] {
                    let mut button = right.spawn((
                        Button,
                        Node {
                            width: Val::Px(40.),
                            height: Val::Px(TITLEBAR_HEIGHT),
                            border: UiRect::all(Val::Px(1.)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(if marker == 0 { initial_pin_bg } else { NORMAL_BG }),
                        BorderColor::all(Color::srgba(0.42, 0.44, 0.48, 0.9)),
                    ));
                    match marker {
                        0 => { button.insert(PushpinButton); }
                        1 => { button.insert(MinimizeButton); }
                        2 => { button.insert(MaximizeButton); }
                        _ => { button.insert(CloseButton); }
                    }
                    button.with_child((
                        Node {
                            width: Val::Px(15.),
                            height: Val::Px(15.),
                            ..default()
                        },
                        ImageNode::new(icon),
                    ));
                    let hint = match marker {
                        0 => "置顶",
                        1 => "最小化",
                        2 => "最大化",
                        _ => "关闭",
                    };
                    button.with_children(|button| {
                        spawn_tooltip(button, ui_font.clone(), hint);
                    });
                }
            });
    });

    // Mask content container
    let mask_entity = commands
        .spawn((
            Node {
                flex_grow: 1.,
                ..default()
            },
            MaskContentMarker,
        ))
        .id();
    commands.insert_resource(MaskContentEntity(mask_entity));

    let toolbar_entity = commands
        .spawn((
            Node {
                width: Val::Px(42.),
                height: Val::Percent(100.),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.),
                padding: UiRect::px(0., 0., 4., 4.),
                ..default()
            },
            BackgroundColor(titlebar_bg),
        ))
        .id();

    commands.entity(toolbar_entity).with_children(|toolbar| {
        for action in [
            DeviceAction::ScreenOff,
            DeviceAction::ScreenOn,
            DeviceAction::VolumeDown,
            DeviceAction::VolumeUp,
            DeviceAction::Back,
            DeviceAction::Home,
            DeviceAction::AppSwitch,
        ] {
            let icon = match action {
                DeviceAction::ScreenOff => screen_off_icon.clone(),
                DeviceAction::ScreenOn => screen_on_icon.clone(),
                DeviceAction::VolumeDown => volume_down_icon.clone(),
                DeviceAction::VolumeUp => volume_up_icon.clone(),
                DeviceAction::Back => back_icon.clone(),
                DeviceAction::Home => home_icon.clone(),
                DeviceAction::AppSwitch => menu_icon.clone(),
            };
            toolbar
                .spawn((
                    Button,
                    Node {
                        width: Val::Px(34.),
                        height: Val::Px(32.),
                        border: UiRect::all(Val::Px(1.)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(NORMAL_BG),
                    BorderColor::all(Color::srgba(0.42, 0.44, 0.48, 0.9)),
                    DeviceButton(action),
                ))
                .with_child((
                    Node {
                        width: Val::Px(18.),
                        height: Val::Px(18.),
                        ..default()
                    },
                    ImageNode::new(icon),
                ))
                .with_children(|button| {
                    let hint = match action {
                        DeviceAction::ScreenOff => "息屏",
                        DeviceAction::ScreenOn => "亮屏",
                        DeviceAction::VolumeDown => "音量-",
                        DeviceAction::VolumeUp => "音量+",
                        DeviceAction::Back => "返回",
                        DeviceAction::Home => "主页",
                        DeviceAction::AppSwitch => "多任务",
                    };
                    spawn_tooltip(button, ui_font.clone(), hint);
                });
        }
    });

    let body_entity = commands
        .spawn((
            Node {
                width: Val::Percent(100.),
                flex_grow: 1.,
                flex_direction: FlexDirection::Row,
                min_height: Val::Px(0.),
                display: Display::None,
                ..default()
            },
            ProjectionBodyMarker,
        ))
        .id();

    // Parent hierarchy: titlebar on top; video and device toolbar below.
    commands
        .entity(root_entity)
        .add_children(&[titlebar_entity, body_entity]);
    commands
        .entity(body_entity)
        .add_children(&[mask_entity, toolbar_entity]);

    // Add children to MaskContent
    commands.entity(mask_entity).with_children(|content| {
        // Video (absolute, behind border)
        content.spawn((
            Node {
                width: Val::Percent(100.),
                height: Val::Percent(100.),
                position_type: PositionType::Absolute,
                padding: UiRect::all(Val::Px(BORDER_THICKNESS)),
                box_sizing: BoxSizing::BorderBox,
                display: Display::None,
                ..default()
            },
            ZIndex(-1),
            BackgroundColor(Color::NONE),
            MaterialNode(video_material),
            VideoPlayer,
        ));

        // Border (absolute to fill the content area)
        content.spawn((
            Node {
                width: Val::Percent(100.),
                height: Val::Percent(100.),
                position_type: PositionType::Absolute,
                border: UiRect::all(Val::Px(BORDER_THICKNESS)),
                box_sizing: BoxSizing::BorderBox,
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderColor::all(border_color),
        ));

        // Resize handles (invisible, layered on top of border)
        let edge_z = ZIndex(10);
        let corner_z = ZIndex(11);

        // Edge handles
        content.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.),
                left: Val::Px(0.),
                width: Val::Percent(100.),
                height: Val::Px(RESIZE_HANDLE_SIZE),
                ..default()
            },
            edge_z,
            BackgroundColor(Color::NONE),
            Interaction::default(),
            ResizeHandle(CompassOctant::North),
        ));
        content.spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(0.),
                left: Val::Px(0.),
                width: Val::Percent(100.),
                height: Val::Px(RESIZE_HANDLE_SIZE),
                ..default()
            },
            edge_z,
            BackgroundColor(Color::NONE),
            Interaction::default(),
            ResizeHandle(CompassOctant::South),
        ));
        content.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.),
                left: Val::Px(0.),
                width: Val::Px(RESIZE_HANDLE_SIZE),
                height: Val::Percent(100.),
                ..default()
            },
            edge_z,
            BackgroundColor(Color::NONE),
            Interaction::default(),
            ResizeHandle(CompassOctant::West),
        ));
        content.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.),
                right: Val::Px(0.),
                width: Val::Px(RESIZE_HANDLE_SIZE),
                height: Val::Percent(100.),
                ..default()
            },
            edge_z,
            BackgroundColor(Color::NONE),
            Interaction::default(),
            ResizeHandle(CompassOctant::East),
        ));

        // Corner handles (higher z-index to capture clicks over edges)
        content.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.),
                left: Val::Px(0.),
                width: Val::Px(RESIZE_HANDLE_SIZE),
                height: Val::Px(RESIZE_HANDLE_SIZE),
                ..default()
            },
            corner_z,
            BackgroundColor(Color::NONE),
            Interaction::default(),
            ResizeHandle(CompassOctant::NorthWest),
        ));
        content.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.),
                right: Val::Px(0.),
                width: Val::Px(RESIZE_HANDLE_SIZE),
                height: Val::Px(RESIZE_HANDLE_SIZE),
                ..default()
            },
            corner_z,
            BackgroundColor(Color::NONE),
            Interaction::default(),
            ResizeHandle(CompassOctant::NorthEast),
        ));
        content.spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(0.),
                left: Val::Px(0.),
                width: Val::Px(RESIZE_HANDLE_SIZE),
                height: Val::Px(RESIZE_HANDLE_SIZE),
                ..default()
            },
            corner_z,
            BackgroundColor(Color::NONE),
            Interaction::default(),
            ResizeHandle(CompassOctant::SouthWest),
        ));
        content.spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(0.),
                right: Val::Px(0.),
                width: Val::Px(RESIZE_HANDLE_SIZE),
                height: Val::Px(RESIZE_HANDLE_SIZE),
                ..default()
            },
            corner_z,
            BackgroundColor(Color::NONE),
            Interaction::default(),
            ResizeHandle(CompassOctant::SouthEast),
        ));
    });
}

fn spawn_tooltip(parent: &mut ChildSpawnerCommands, font: Handle<Font>, label: &str) {
    parent.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(34.),
            top: Val::Px(0.),
            min_width: Val::Px(54.),
            padding: UiRect::axes(Val::Px(8.), Val::Px(4.)),
            border_radius: BorderRadius::all(Val::Px(4.)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::srgba(0.04, 0.04, 0.045, 0.98)),
        ZIndex(100),
        TooltipText,
    ))
    .with_child((
        Text::new(label),
        TextFont {
            font: font.into(),
            font_size: FontSize::Px(12.),
            ..default()
        },
        TextColor(Color::WHITE),
    ));
}

fn handle_titlebar_drag(
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    interaction_query: Query<&Interaction, (With<TitlebarMarker>, Changed<Interaction>)>,
    button_query: Query<
        &Interaction,
        Or<(
            With<MinimizeButton>,
            With<MaximizeButton>,
            With<PushpinButton>,
            With<CloseButton>,
            With<DeviceButton>,
        )>,
    >,
) {
    let button_pressed = button_query.iter().any(|i| *i == Interaction::Pressed);
    if !button_pressed && interaction_query.iter().any(|i| *i == Interaction::Pressed) {
        window.start_drag_move();
    }
}

fn handle_titlebar_buttons(
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut maximized: Local<bool>,
    minimize_query: Query<&Interaction, (With<MinimizeButton>, Changed<Interaction>)>,
    maximize_query: Query<&Interaction, (With<MaximizeButton>, Changed<Interaction>)>,
    pushpin_query: Query<&Interaction, (With<PushpinButton>, Changed<Interaction>)>,
    close_query: Query<&Interaction, (With<CloseButton>, Changed<Interaction>)>,
    d_tx: Res<ChannelSenderD>,
) {
    for interaction in minimize_query.iter() {
        if *interaction == Interaction::Pressed {
            window.set_minimized(true);
        }
    }
    for interaction in maximize_query.iter() {
        if *interaction == Interaction::Pressed {
            *maximized = !*maximized;
            window.set_maximized(*maximized);
        }
    }
    for interaction in pushpin_query.iter() {
        if *interaction == Interaction::Pressed {
            let top = window.window_level != WindowLevel::AlwaysOnTop;
            if top {
                window.window_level = WindowLevel::AlwaysOnTop;
            } else {
                window.window_level = WindowLevel::Normal;
            }
            LocalConfig::set_always_on_top(top);
        }
    }
    for interaction in close_query.iter() {
        if *interaction == Interaction::Pressed {
            if let Some(device) = ControlledDevice::get_main_device_blocking() {
                let _ = d_tx
                    .0
                    .send(ControllerCommand::ShutdownMain(device.scid.clone()));
            }
        }
    }
}

fn handle_device_buttons(
    query: Query<(&DeviceButton, &Interaction), Changed<Interaction>>,
    cs_tx: Res<ChannelSenderCS>,
) {
    for (btn, interaction) in query.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn.0 {
            DeviceAction::Back => device_action::inject_keycode(&cs_tx.0, Keycode::Back),
            DeviceAction::Home => device_action::inject_keycode(&cs_tx.0, Keycode::Home),
            DeviceAction::AppSwitch => device_action::inject_keycode(&cs_tx.0, Keycode::AppSwitch),
            DeviceAction::ScreenOff => device_action::set_display_power(&cs_tx.0, false),
            DeviceAction::ScreenOn => device_action::set_display_power(&cs_tx.0, true),
            DeviceAction::VolumeUp => device_action::inject_keycode(&cs_tx.0, Keycode::VolumeUp),
            DeviceAction::VolumeDown => {
                device_action::inject_keycode(&cs_tx.0, Keycode::VolumeDown)
            }
        }
    }
}

const NORMAL_BG: Color = Color::srgba(0.28, 0.30, 0.34, 0.96);
const HOVERED_BG: Color = Color::srgba(0.46, 0.48, 0.53, 0.98);
const PRESSED_BG: Color = Color::srgba(0.15, 0.15, 0.15, 0.85);

const CLOSE_HOVER_BG: Color = Color::srgba(0.77, 0.12, 0.12, 0.95);
const CLOSE_PRESSED_BG: Color = Color::srgba(0.60, 0.06, 0.06, 1.0);
const PIN_ACTIVE_BG: Color = Color::srgba(0.20, 0.45, 0.65, 0.85);

fn button_interaction(
    window: Single<&Window, With<PrimaryWindow>>,
    minimize_query: Query<(Entity, &Interaction), (With<MinimizeButton>, Changed<Interaction>)>,
    maximize_query: Query<(Entity, &Interaction), (With<MaximizeButton>, Changed<Interaction>)>,
    pushpin_query: Query<(Entity, &Interaction), (With<PushpinButton>, Changed<Interaction>)>,
    close_query: Query<(Entity, &Interaction), (With<CloseButton>, Changed<Interaction>)>,
    device_btn_query: Query<(Entity, &Interaction), (With<DeviceButton>, Changed<Interaction>)>,
    mut bg_query: Query<&mut BackgroundColor>,
) {
    for (entity, interaction) in minimize_query.iter() {
        if let Ok(mut bg) = bg_query.get_mut(entity) {
            *bg = match *interaction {
                Interaction::Pressed => PRESSED_BG,
                Interaction::Hovered => HOVERED_BG,
                Interaction::None => NORMAL_BG,
            }
            .into();
        }
    }
    for (entity, interaction) in maximize_query.iter() {
        if let Ok(mut bg) = bg_query.get_mut(entity) {
            *bg = match *interaction {
                Interaction::Pressed => PRESSED_BG,
                Interaction::Hovered => HOVERED_BG,
                Interaction::None => NORMAL_BG,
            }
            .into();
        }
    }
    let pinned = window.window_level == WindowLevel::AlwaysOnTop;
    for (entity, interaction) in pushpin_query.iter() {
        if let Ok(mut bg) = bg_query.get_mut(entity) {
            *bg = if pinned {
                match *interaction {
                    Interaction::Pressed => PRESSED_BG,
                    Interaction::Hovered => HOVERED_BG,
                    Interaction::None => PIN_ACTIVE_BG,
                }
            } else {
                match *interaction {
                    Interaction::Pressed => PRESSED_BG,
                    Interaction::Hovered => HOVERED_BG,
                    Interaction::None => NORMAL_BG,
                }
            }
            .into();
        }
    }
    for (entity, interaction) in close_query.iter() {
        if let Ok(mut bg) = bg_query.get_mut(entity) {
            *bg = match *interaction {
                Interaction::Pressed => CLOSE_PRESSED_BG,
                Interaction::Hovered => CLOSE_HOVER_BG,
                Interaction::None => NORMAL_BG,
            }
            .into();
        }
    }
    for (entity, interaction) in device_btn_query.iter() {
        if let Ok(mut bg) = bg_query.get_mut(entity) {
            *bg = match *interaction {
                Interaction::Pressed => PRESSED_BG,
                Interaction::Hovered => HOVERED_BG,
                Interaction::None => NORMAL_BG,
            }
            .into();
        }
    }
}

fn sync_tooltips(
    buttons: Query<
        (&Interaction, &Children),
        Or<(
            With<MinimizeButton>,
            With<MaximizeButton>,
            With<PushpinButton>,
            With<CloseButton>,
            With<DeviceButton>,
        )>,
    >,
    mut tooltip_nodes: Query<&mut Node, With<TooltipText>>,
) {
    for (interaction, children) in buttons.iter() {
        for child in children.iter() {
            if let Ok(mut node) = tooltip_nodes.get_mut(child) {
                node.display = if *interaction == Interaction::Hovered {
                    Display::Flex
                } else {
                    Display::None
                };
            }
        }
    }
}

fn cursor_for_resize_direction(direction: CompassOctant) -> SystemCursorIcon {
    match direction {
        CompassOctant::North => SystemCursorIcon::NResize,
        CompassOctant::NorthEast => SystemCursorIcon::NeResize,
        CompassOctant::East => SystemCursorIcon::EResize,
        CompassOctant::SouthEast => SystemCursorIcon::SeResize,
        CompassOctant::South => SystemCursorIcon::SResize,
        CompassOctant::SouthWest => SystemCursorIcon::SwResize,
        CompassOctant::West => SystemCursorIcon::WResize,
        CompassOctant::NorthWest => SystemCursorIcon::NwResize,
    }
}

fn resize_handle_priority(handle: CompassOctant) -> u8 {
    match handle {
        CompassOctant::NorthEast
        | CompassOctant::NorthWest
        | CompassOctant::SouthEast
        | CompassOctant::SouthWest => 1,
        CompassOctant::North | CompassOctant::East | CompassOctant::South | CompassOctant::West => {
            0
        }
    }
}

fn handle_resize(
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut resize_state: ResMut<MaskResizeState>,
    query: Query<(&ResizeHandle, &Interaction), Changed<Interaction>>,
) {
    let Some(handle) = query
        .iter()
        .filter_map(|(handle, interaction)| {
            (*interaction == Interaction::Pressed).then_some(handle.0)
        })
        .max_by_key(|handle| resize_handle_priority(*handle))
    else {
        return;
    };

    resize_state.begin_interaction();
    window.start_drag_resize(handle);
}

fn active_resize_handle(
    resize_query: Query<(&ResizeHandle, &Interaction)>,
) -> Option<CompassOctant> {
    resize_query
        .iter()
        .filter_map(|(handle, interaction)| {
            matches!(*interaction, Interaction::Hovered | Interaction::Pressed).then_some(handle.0)
        })
        .max_by_key(|handle| resize_handle_priority(*handle))
}

fn sync_resize_cursor(
    resize_query: Query<(&ResizeHandle, &Interaction)>,
    mut cursor_query: Single<&mut CursorIcon, With<PrimaryWindow>>,
) {
    let resize_cursor = active_resize_handle(resize_query)
        .map(cursor_for_resize_direction)
        .unwrap_or(SystemCursorIcon::Default);

    let next_cursor = CursorIcon::from(resize_cursor);
    if **cursor_query != next_cursor {
        **cursor_query = next_cursor;
    }
}

fn sync_titlebar_visibility(
    titlebar_state: Res<TitlebarState>,
    mut titlebar_query: Query<&mut Node, With<TitlebarMarker>>,
) {
    if !titlebar_state.is_changed() {
        return;
    }
    for mut node in titlebar_query.iter_mut() {
        node.display = if titlebar_state.visible {
            Display::Flex
        } else {
            Display::None
        };
    }
}

fn sync_titlebar_title_visibility(
    window: Single<&Window, (With<PrimaryWindow>, Changed<Window>)>,
    mut title_query: Query<&mut Node, With<TitlebarTitleMarker>>,
) {
    let display = if window.resolution.width() >= TITLEBAR_TITLE_MIN_WINDOW_WIDTH {
        Display::Flex
    } else {
        Display::None
    };

    for mut node in title_query.iter_mut() {
        if node.display != display {
            node.display = display;
        }
    }
}

fn sync_pushpin_style(
    window: Single<&Window, (With<PrimaryWindow>, Changed<Window>)>,
    mut pushpin_query: Query<(&Interaction, &mut BackgroundColor), With<PushpinButton>>,
) {
    let pinned = window.window_level == WindowLevel::AlwaysOnTop;
    for (interaction, mut bg) in pushpin_query.iter_mut() {
        if *interaction == Interaction::None {
            *bg = if pinned {
                PIN_ACTIVE_BG
            } else {
                NORMAL_BG
            }
            .into();
        }
    }
}
