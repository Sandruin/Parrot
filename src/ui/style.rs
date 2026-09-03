use egui::{Color32, CornerRadius, Stroke, Visuals};

/// Applies the app theme: rounded flat widgets, generous spacing, icon font registered.
pub fn apply(ctx: &egui::Context) {
    egui_material_icons::initialize(ctx);

    ctx.style_mut_of(egui::Theme::Light, |style| tune(style, Visuals::light()));
    ctx.style_mut_of(egui::Theme::Dark, |style| tune(style, Visuals::dark()));
}

fn tune(style: &mut egui::Style, mut visuals: Visuals) {
    let dark = visuals.dark_mode;
    let radius = CornerRadius::same(6);
    let accent = if dark { Color32::from_rgb(96, 165, 250) } else { Color32::from_rgb(37, 99, 235) };

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

    if dark {
        visuals.panel_fill = Color32::from_rgb(24, 26, 31);
        visuals.window_fill = Color32::from_rgb(30, 33, 39);
        visuals.extreme_bg_color = Color32::from_rgb(17, 18, 22);
        visuals.faint_bg_color = Color32::from_rgb(30, 33, 39);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(40, 44, 52);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(52, 57, 67);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(62, 68, 80);
    } else {
        visuals.panel_fill = Color32::from_rgb(246, 247, 249);
        visuals.window_fill = Color32::WHITE;
        visuals.extreme_bg_color = Color32::WHITE;
        visuals.faint_bg_color = Color32::from_rgb(240, 242, 245);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(228, 231, 236);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(218, 222, 229);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(206, 211, 220);
    }

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(12);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.interact_size.y = 26.0;
}
