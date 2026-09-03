use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{Color32, CornerRadius, FontData, FontFamily, FontId, Shadow, Stroke, TextStyle, Visuals};

/// Family name of the bundled medium weight face, used for headers and toolbar captions.
pub const MEDIUM_FAMILY: &str = "inter-medium";

/// Applies the app theme: bundled Inter font, icon font, rounded flat widgets, generous spacing.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    egui_material_icons::initialize(ctx);

    ctx.style_mut_of(egui::Theme::Light, |style| tune(style, Visuals::light()));
    ctx.style_mut_of(egui::Theme::Dark, |style| tune(style, Visuals::dark()));
}

/// Font id of the bundled medium weight face at `size`.
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM_FAMILY.into()))
}

/// Frame for the toolbar and status panels, lifted slightly off the list background.
pub fn chrome_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::side_top_panel(style)
        .fill(style.visuals.window_fill)
        .inner_margin(egui::Margin::symmetric(10, 4))
}

/// Accent colour of the current theme.
pub fn accent(visuals: &Visuals) -> Color32 {
    if visuals.dark_mode { Color32::from_rgb(96, 165, 250) } else { Color32::from_rgb(37, 99, 235) }
}

/// Colour used for the record button and the recording state.
pub fn record_red(visuals: &Visuals) -> Color32 {
    if visuals.dark_mode { Color32::from_rgb(248, 113, 113) } else { Color32::from_rgb(220, 38, 38) }
}

/// Colour used for the play button and successful outcomes.
pub fn play_green(visuals: &Visuals) -> Color32 {
    if visuals.dark_mode { Color32::from_rgb(74, 222, 128) } else { Color32::from_rgb(22, 163, 74) }
}

fn install_fonts(ctx: &egui::Context) {
    ctx.add_font(FontInsert::new(
        "inter",
        FontData::from_static(include_bytes!("../../assets/Inter-Regular.ttf")),
        vec![InsertFontFamily { family: FontFamily::Proportional, priority: FontPriority::Highest }],
    ));
    ctx.add_font(FontInsert::new(
        MEDIUM_FAMILY,
        FontData::from_static(include_bytes!("../../assets/Inter-Medium.ttf")),
        vec![InsertFontFamily {
            family: FontFamily::Name(MEDIUM_FAMILY.into()),
            priority: FontPriority::Highest,
        }],
    ));
}

fn tune(style: &mut egui::Style, mut visuals: Visuals) {
    let dark = visuals.dark_mode;
    let radius = CornerRadius::same(6);
    let accent = accent(&visuals);

    visuals.widgets.noninteractive.corner_radius = radius;
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.open.corner_radius = radius;
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);

    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent.gamma_multiply(0.6));
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent);
    visuals.selection.bg_fill = accent.gamma_multiply(if dark { 0.45 } else { 0.25 });
    visuals.selection.stroke = Stroke::new(1.0, accent);
    visuals.hyperlink_color = accent;
    visuals.window_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 120 } else { 40 }),
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 100 } else { 30 }),
    };

    if dark {
        visuals.panel_fill = Color32::from_rgb(24, 26, 31);
        visuals.window_fill = Color32::from_rgb(30, 33, 39);
        visuals.extreme_bg_color = Color32::from_rgb(17, 18, 22);
        visuals.faint_bg_color = Color32::from_rgb(31, 34, 40);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(48, 52, 61));
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(40, 44, 52);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(52, 57, 67);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(62, 68, 80);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(222, 226, 234));
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(222, 226, 234));
    } else {
        visuals.panel_fill = Color32::from_rgb(246, 247, 249);
        visuals.window_fill = Color32::WHITE;
        visuals.extreme_bg_color = Color32::WHITE;
        visuals.faint_bg_color = Color32::from_rgb(240, 242, 245);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(220, 224, 230));
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(228, 231, 236);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(218, 222, 229);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(206, 211, 220);
    }

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(14);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.interact_size.y = 26.0;
    style.spacing.combo_width = 130.0;
    style.spacing.slider_width = 160.0;
    style.spacing.icon_width = 18.0;

    style.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.5)),
        (TextStyle::Button, FontId::proportional(13.5)),
        (TextStyle::Heading, FontId::new(17.0, FontFamily::Name(MEDIUM_FAMILY.into()))),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
}
