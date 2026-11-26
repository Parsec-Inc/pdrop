use super::theme::{ICONS, S, snap};
use eframe::egui::{
    self, Color32, ColorImage, Mesh, Pos2, Rect, Shape, TextureHandle, TextureOptions, pos2, vec2,
};

pub struct Sheet {
    pub(super) tex: TextureHandle,
    n: usize,
}

impl Sheet {
    fn load(ctx: &egui::Context, name: &str, png: &[u8], n: usize) -> Self {
        Self::load_with(ctx, name, png, n, TextureOptions::NEAREST)
    }

    fn load_with(
        ctx: &egui::Context,
        name: &str,
        png: &[u8],
        n: usize,
        opt: TextureOptions,
    ) -> Self {
        let im = image::load_from_memory(png)
            .expect("sprite embutido inválido")
            .to_rgba8();
        let size = [im.width() as usize, im.height() as usize];
        let tex = ctx.load_texture(
            name,
            ColorImage::from_rgba_unmultiplied(size, im.as_raw()),
            opt,
        );
        Self { tex, n }
    }

    pub(super) fn nine(&self, i: usize, r: Rect, m: f32, tint: Color32) -> Shape {
        let (fw, fh) = (
            self.tex.size()[0] as f32 / self.n as f32,
            self.tex.size()[1] as f32,
        );
        let (u0, du) = (i as f32 / self.n as f32, 1.0 / self.n as f32);
        let r = Rect::from_min_max(snap(r.min), snap(r.max));
        let xs = [r.left(), r.left() + m * S, r.right() - m * S, r.right()];
        let ys = [r.top(), r.top() + m * S, r.bottom() - m * S, r.bottom()];
        let us = [u0, u0 + du * m / fw, u0 + du * (fw - m) / fw, u0 + du];
        let vs = [0.0, m / fh, (fh - m) / fh, 1.0];
        let mut mesh = Mesh::with_texture(self.tex.id());
        for y in 0..3 {
            for x in 0..3 {
                let rr = Rect::from_min_max(pos2(xs[x], ys[y]), pos2(xs[x + 1], ys[y + 1]));
                let uv = Rect::from_min_max(pos2(us[x], vs[y]), pos2(us[x + 1], vs[y + 1]));
                mesh.add_rect_with_uv(rr, uv, tint);
            }
        }
        Shape::mesh(mesh)
    }

    pub(super) fn draw(&self, p: &egui::Painter, i: usize, at: Pos2, scale: f32, tint: Color32) {
        let n = self.n as f32;
        let i = (i % self.n) as f32;
        let size = self.tex.size_vec2() * vec2(1.0 / n, 1.0) * scale;
        let r = egui::Rect::from_center_size(pos2(at.x.round(), at.y.round()), size);
        let uv = egui::Rect::from_min_max(pos2(i / n, 0.0), pos2((i + 1.0) / n, 1.0));
        p.image(self.tex.id(), r, uv, tint);
    }
}

pub struct Sprites {
    pub(super) icons: Sheet,
    pub(super) ping: Sheet,
    pub(super) transfer: Sheet,
    pub(super) drop: Sheet,
    pub(super) logo: Sheet,
    pub(super) ui: Sheet,
    pub(super) actions: Sheet,
    pub(super) bg: Sheet,
    pub(super) radar: Option<TextureHandle>,
}

impl Sprites {
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            icons: Sheet::load(
                ctx,
                "icons",
                include_bytes!("../../assets/icons.png"),
                ICONS.len(),
            ),
            ping: Sheet::load(ctx, "ping", include_bytes!("../../assets/ping.png"), 6),
            transfer: Sheet::load(
                ctx,
                "transfer",
                include_bytes!("../../assets/transfer.png"),
                4,
            ),
            drop: Sheet::load(ctx, "drop", include_bytes!("../../assets/drop.png"), 5),
            logo: Sheet::load(ctx, "logo", include_bytes!("../../assets/logo.png"), 1),
            ui: Sheet::load(ctx, "ui", include_bytes!("../../assets/ui.png"), 9),
            actions: Sheet::load(
                ctx,
                "actions",
                include_bytes!("../../assets/actions.png"),
                7,
            ),
            bg: Sheet::load_with(
                ctx,
                "bg",
                include_bytes!("../../assets/bg.png"),
                1,
                TextureOptions::NEAREST_REPEAT,
            ),
            radar: None,
        }
    }
}
