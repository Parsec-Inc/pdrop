use eframe::egui::{self, Color32, Pos2, RichText, Stroke, pos2, vec2};

pub type Rgb = [u8; 3];

pub(super) const BG: Rgb = [17, 16, 26];

pub(super) const FG: Rgb = [236, 230, 222];

pub(super) const MUTED: Rgb = [132, 127, 160];

pub(super) const DIM: Rgb = [70, 66, 100];

pub(super) const OK: Rgb = [130, 214, 170];

pub(super) const WARN: Rgb = [255, 190, 90];

pub(super) const DISC: Rgb = [12, 11, 19];

pub(super) const GRID: Rgb = [30, 28, 46];

pub(super) const INK: Rgb = [8, 7, 15];

pub const COLORS: [(&str, Rgb); 8] = [
    ("coral", [255, 112, 102]),
    ("âmbar", [227, 154, 7]),
    ("lima", [148, 187, 59]),
    ("verde", [35, 201, 135]),
    ("turquesa", [12, 195, 195]),
    ("azul", [84, 179, 253]),
    ("violeta", [178, 150, 255]),
    ("magenta", [240, 126, 194]),
];

pub const ICONS: [&str; 11] = [
    "ponto",
    "gato",
    "robô",
    "laptop",
    "fantasma",
    "peixe",
    "cobra",
    "coruja",
    "coração",
    "estrela",
    "alien",
];

pub(super) const S: f32 = 2.0;

pub(super) const TXT: f32 = 22.0;

pub(super) fn c(a: Rgb) -> Color32 {
    Color32::from_rgb(a[0], a[1], a[2])
}

pub(super) fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let m = |i: usize| (a[i] as f64 * (1.0 - t) + b[i] as f64 * t) as u8;
    [m(0), m(1), m(2)]
}

pub(super) fn mix(a: Rgb, b: Rgb, t: f64) -> Color32 {
    c(lerp(a, b, t))
}

pub(super) fn txt(s: impl Into<String>, col: Rgb) -> RichText {
    RichText::new(s).color(c(col))
}

pub(super) fn snap(p: Pos2) -> Pos2 {
    pos2((p.x / S).round() * S, (p.y / S).round() * S)
}

pub fn setup(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let tweak = egui::FontTweak {
        hinting: Some(false),
        subpixel_binning: Some(false),
        ..Default::default()
    };
    fonts.font_data.insert(
        "pdrop".into(),
        std::sync::Arc::new(
            egui::FontData::from_static(include_bytes!("../../assets/pdrop.ttf")).tweak(tweak),
        ),
    );
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(fam)
            .or_default()
            .insert(0, "pdrop".into());
    }
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|s| {
        for f in s.text_styles.values_mut() {
            f.size = TXT;
        }
        s.spacing.item_spacing = vec2(8.0, 6.0);
    });

    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = c(DISC);
    v.window_stroke = Stroke::new(S, c(DIM));
    v.window_corner_radius = egui::CornerRadius::ZERO;
    v.menu_corner_radius = egui::CornerRadius::ZERO;
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = egui::CornerRadius::ZERO;
    }
    v.extreme_bg_color = c(INK);
    v.override_text_color = Some(c(FG));
    v.selection.bg_fill = mix(BG, COLORS[0].1, 0.45);
    v.text_cursor.stroke = Stroke::new(S, c(FG));
    ctx.set_visuals(v);
}
