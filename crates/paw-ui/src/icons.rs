//! Ícones da suíte, todos no mesmo estilo: contorno de traço uniforme num grid de 24 × 24
//! (como os ícones de linha do Resolve e do Krita), cantos arredondados e pontas redondas.
//! Só os controles de transporte (tocar, pausar, parar, gravar) são cheios, como em todo
//! programa de áudio e vídeo.
//!
//! Desenhados com o painter (sem fonte de emoji nem arquivos de imagem), então ficam nítidos em
//! qualquer tamanho. Para um ícone novo: desenhe no grid de 24 com `Pen`, margem de 2 unidades.

use std::f32::consts::{PI, TAU};

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, vec2};

use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    // Edição de vídeo e áudio
    Select,
    Blade,
    Magnet,
    Link,
    Lock,
    Unlock,
    Monitor,
    Speaker,
    Play,
    Pause,
    Stop,
    PrevFrame,
    NextFrame,
    ToStart,
    ToEnd,
    Reverse,
    Import,
    Plus,
    Minus,
    Fit,
    MediaPool,
    Effects,
    Inspector,
    DeliverPage,
    Insert,
    Overwrite,
    Append,
    Skim,
    Trim,
    Loop,
    Record,
    Envelope,
    IBeam,
    // Pintura
    Brush,
    Eraser,
    Smudge,
    Clone,
    Bucket,
    Gradient,
    Eyedropper,
    SelRect,
    SelEllipse,
    Lasso,
    Wand,
    Move,
    Transform,
    Crop,
    Line,
    Rectangle,
    Ellipse,
    Text,
    Layers,
    FlipH,
    FlipV,
    Swap,
    // Gerais
    Folder,
    FolderOpen,
    Save,
    Duplicate,
    Trash,
    Close,
    MergeDown,
    ChevronUp,
    ChevronDown,
    Undo,
    Redo,
    Scissors,
    Copy,
    Paste,
    ZoomIn,
    ZoomOut,
    Search,
    Gear,
    Eye,
    EyeOff,
    Mic,
    Sparkles,
    Waveform,
    Document,
    Book,
    Outline,
    Keyframe,
    Reset,
    Wheel,
    Check,
    /// Marcar entrada / saída (colchetes).
    MarkIn,
    MarkOut,
}

impl Icon {
    /// Todos, na ordem da declaração (para catálogos e testes).
    pub const ALL: &'static [Icon] = &[
        Icon::Select,
        Icon::Blade,
        Icon::Magnet,
        Icon::Link,
        Icon::Lock,
        Icon::Unlock,
        Icon::Monitor,
        Icon::Speaker,
        Icon::Play,
        Icon::Pause,
        Icon::Stop,
        Icon::PrevFrame,
        Icon::NextFrame,
        Icon::ToStart,
        Icon::ToEnd,
        Icon::Reverse,
        Icon::Import,
        Icon::Plus,
        Icon::Minus,
        Icon::Fit,
        Icon::MediaPool,
        Icon::Effects,
        Icon::Inspector,
        Icon::DeliverPage,
        Icon::Insert,
        Icon::Overwrite,
        Icon::Append,
        Icon::Skim,
        Icon::Trim,
        Icon::Loop,
        Icon::Record,
        Icon::Envelope,
        Icon::IBeam,
        Icon::Brush,
        Icon::Eraser,
        Icon::Smudge,
        Icon::Clone,
        Icon::Bucket,
        Icon::Gradient,
        Icon::Eyedropper,
        Icon::SelRect,
        Icon::SelEllipse,
        Icon::Lasso,
        Icon::Wand,
        Icon::Move,
        Icon::Transform,
        Icon::Crop,
        Icon::Line,
        Icon::Rectangle,
        Icon::Ellipse,
        Icon::Text,
        Icon::Layers,
        Icon::FlipH,
        Icon::FlipV,
        Icon::Swap,
        Icon::Folder,
        Icon::FolderOpen,
        Icon::Save,
        Icon::Duplicate,
        Icon::Trash,
        Icon::Close,
        Icon::MergeDown,
        Icon::ChevronUp,
        Icon::ChevronDown,
        Icon::Undo,
        Icon::Redo,
        Icon::Scissors,
        Icon::Copy,
        Icon::Paste,
        Icon::ZoomIn,
        Icon::ZoomOut,
        Icon::Search,
        Icon::Gear,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Mic,
        Icon::Sparkles,
        Icon::Waveform,
        Icon::Document,
        Icon::Book,
        Icon::Outline,
        Icon::Keyframe,
        Icon::Reset,
        Icon::Wheel,
        Icon::Check,
        Icon::MarkIn,
        Icon::MarkOut,
    ];
}

/// Caneta no grid de 24 × 24 dentro de `r`.
struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    c: Color32,
    w: f32,
}

impl Pen<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        let k = self.r.width().min(self.r.height()) / 24.0;
        let o = self.r.center() - vec2(12.0, 12.0) * k;
        o + vec2(x, y) * k
    }

    fn k(&self) -> f32 {
        self.r.width().min(self.r.height()) / 24.0
    }

    fn stroke(&self) -> Stroke {
        Stroke::new(self.w, self.c)
    }

    /// Ponta redonda: um disco do tamanho do traço em cada extremidade.
    fn cap(&self, q: Pos2) {
        if self.w > 1.4 {
            self.p.circle_filled(q, self.w * 0.5, self.c);
        }
    }

    fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32) {
        let (a, b) = (self.pt(x1, y1), self.pt(x2, y2));
        self.p.line_segment([a, b], self.stroke());
        self.cap(a);
        self.cap(b);
    }

    fn path(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        for q in &v {
            self.cap(*q);
        }
        self.p.add(Shape::line(v, self.stroke()));
    }

    fn closed(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        for q in &v {
            self.cap(*q);
        }
        self.p.add(Shape::closed_line(v, self.stroke()));
    }

    /// Polígono cheio (convexo).
    fn fill(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(Shape::convex_polygon(v, self.c, Stroke::NONE));
    }

    fn circle(&self, x: f32, y: f32, r: f32) {
        self.p
            .circle_stroke(self.pt(x, y), r * self.k(), self.stroke());
    }

    fn disc(&self, x: f32, y: f32, r: f32) {
        self.p.circle_filled(self.pt(x, y), r * self.k(), self.c);
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32, rad: f32) {
        let r = Rect::from_min_max(self.pt(x, y), self.pt(x + w, y + h));
        self.p
            .rect_stroke(r, rad * self.k(), self.stroke(), egui::StrokeKind::Middle);
    }

    fn frect(&self, x: f32, y: f32, w: f32, h: f32, rad: f32) {
        let r = Rect::from_min_max(self.pt(x, y), self.pt(x + w, y + h));
        self.p.rect_filled(r, rad * self.k(), self.c);
    }

    /// Pontos de um arco de elipse; ângulos em graus, 0 = direita, crescendo no sentido horário.
    fn arc_pts(&self, cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
        let n = (((a1 - a0).abs() / 12.0).ceil() as usize).max(2);
        (0..=n)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                (cx + rx * a.cos(), cy + ry * a.sin())
            })
            .collect()
    }

    fn arc(&self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
        self.path(&self.arc_pts(cx, cy, r, r, a0, a1));
    }

    fn ellipse(&self, cx: f32, cy: f32, rx: f32, ry: f32) {
        let mut v = self.arc_pts(cx, cy, rx, ry, 0.0, 360.0);
        v.pop();
        let v: Vec<Pos2> = v.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(Shape::closed_line(v, self.stroke()));
    }

    /// Caminho tracejado (seleções).
    fn dashed(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        let dash = 2.6 * self.k();
        let mut on = true;
        let mut left = dash;
        for w in v.windows(2) {
            let (mut a, b) = (w[0], w[1]);
            let mut len = (b - a).length();
            let dir = (b - a) / len.max(1e-3);
            while len > 0.0 {
                let step = left.min(len);
                let q = a + dir * step;
                if on {
                    self.p.line_segment([a, q], self.stroke());
                }
                a = q;
                len -= step;
                left -= step;
                if left <= 1e-3 {
                    on = !on;
                    left = dash;
                }
            }
        }
    }

    /// Ponta de seta em `(x, y)` apontando na direção `(dx, dy)`.
    fn head(&self, x: f32, y: f32, dx: f32, dy: f32, size: f32) {
        let l = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (ux, uy) = (dx / l, dy / l);
        let (nx, ny) = (-uy, ux);
        self.path(&[
            (x - ux * size + nx * size, y - uy * size + ny * size),
            (x, y),
            (x - ux * size - nx * size, y - uy * size - ny * size),
        ]);
    }
}

/// Desenha o ícone dentro de `r` (quadrado) na cor `c`.
pub fn paint(painter: &Painter, r: Rect, icon: Icon, c: Color32) {
    let side = r.width().min(r.height());
    let pen = Pen {
        p: painter,
        r,
        c,
        w: (side * 2.0 / 24.0).max(1.15),
    };
    let soft = |a: f32| c.gamma_multiply(a);
    match icon {
        // ---------------------------------------------------------------- transporte (cheios)
        Icon::Play => pen.fill(&[(7.0, 4.5), (19.5, 12.0), (7.0, 19.5)]),
        Icon::Reverse => pen.fill(&[(17.0, 4.5), (4.5, 12.0), (17.0, 19.5)]),
        Icon::Pause => {
            pen.frect(6.0, 5.0, 4.0, 14.0, 1.2);
            pen.frect(14.0, 5.0, 4.0, 14.0, 1.2);
        }
        Icon::Stop => pen.frect(6.0, 6.0, 12.0, 12.0, 2.0),
        Icon::Record => pen.disc(12.0, 12.0, 6.5),
        Icon::PrevFrame => {
            pen.fill(&[(18.0, 6.0), (9.0, 12.0), (18.0, 18.0)]);
            pen.frect(6.0, 6.0, 2.2, 12.0, 0.8);
        }
        Icon::NextFrame => {
            pen.fill(&[(6.0, 6.0), (15.0, 12.0), (6.0, 18.0)]);
            pen.frect(15.8, 6.0, 2.2, 12.0, 0.8);
        }
        Icon::ToStart => {
            pen.frect(4.0, 6.0, 2.2, 12.0, 0.8);
            pen.fill(&[(13.5, 6.0), (6.5, 12.0), (13.5, 18.0)]);
            pen.fill(&[(20.5, 6.0), (13.5, 12.0), (20.5, 18.0)]);
        }
        Icon::ToEnd => {
            pen.frect(17.8, 6.0, 2.2, 12.0, 0.8);
            pen.fill(&[(3.5, 6.0), (10.5, 12.0), (3.5, 18.0)]);
            pen.fill(&[(10.5, 6.0), (17.5, 12.0), (10.5, 18.0)]);
        }
        Icon::Loop => {
            pen.arc(12.0, 12.0, 7.5, -40.0, 250.0);
            let a = (-40.0f32).to_radians();
            let (x, y) = (12.0 + 7.5 * a.cos(), 12.0 + 7.5 * a.sin());
            pen.head(x, y, a.sin(), -a.cos(), 3.2);
        }
        // ---------------------------------------------------------------- edição
        Icon::Select => {
            let pts = [(5.0, 3.5), (19.5, 10.8), (12.8, 12.8), (9.8, 19.8)];
            pen.fill(&[pts[0], pts[1], pts[2]]);
            pen.fill(&[pts[0], pts[2], pts[3]]);
            pen.closed(&pts);
        }
        Icon::Blade => {
            // Estilete: lâmina inclinada com a base reta (como a ferramenta de corte do Resolve).
            pen.path(&[
                (11.0, 16.6),
                (11.0, 19.0),
                (10.0, 20.0),
                (2.5, 20.0),
                (18.4, 3.6),
                (20.0, 3.0),
                (21.2, 4.2),
                (21.0, 6.4),
                (11.7, 15.9),
                (10.3, 15.9),
                (8.2, 13.8),
            ]);
        }
        Icon::Magnet => {
            // U com o arco por baixo (de 180° a 0° passando por 90°) e as marcas dos polos.
            let mut u = vec![(6.0, 4.0), (6.0, 12.0)];
            u.extend(pen.arc_pts(12.0, 12.0, 6.0, 6.0, 180.0, 0.0));
            u.push((18.0, 4.0));
            pen.path(&u);
            pen.line(3.5, 8.0, 8.5, 8.0);
            pen.line(15.5, 8.0, 20.5, 8.0);
        }
        Icon::Link => {
            let mut l = vec![(9.5, 7.0), (7.0, 7.0)];
            l.extend(pen.arc_pts(7.0, 12.0, 5.0, 5.0, 270.0, 90.0));
            l.push((9.5, 17.0));
            pen.path(&l);
            let mut r2 = vec![(14.5, 7.0), (17.0, 7.0)];
            r2.extend(pen.arc_pts(17.0, 12.0, 5.0, 5.0, -90.0, 90.0));
            r2.push((14.5, 17.0));
            pen.path(&r2);
            pen.line(8.5, 12.0, 15.5, 12.0);
        }
        Icon::Lock | Icon::Unlock => {
            pen.rect(4.5, 11.0, 15.0, 10.0, 2.0);
            let mut s = vec![(8.0, 11.0), (8.0, 7.5)];
            let end = if icon == Icon::Lock { 360.0 } else { 320.0 };
            s.extend(pen.arc_pts(12.0, 7.5, 4.0, 4.0, 180.0, end));
            if icon == Icon::Lock {
                s.push((16.0, 11.0));
            }
            pen.path(&s);
            pen.line(12.0, 15.0, 12.0, 17.0);
        }
        Icon::Monitor => {
            pen.rect(2.5, 3.5, 19.0, 13.0, 2.0);
            pen.line(12.0, 16.5, 12.0, 20.5);
            pen.line(8.0, 20.5, 16.0, 20.5);
        }
        Icon::Speaker => {
            pen.closed(&[
                (11.0, 5.0),
                (6.5, 9.0),
                (3.0, 9.0),
                (3.0, 15.0),
                (6.5, 15.0),
                (11.0, 19.0),
            ]);
            pen.arc(13.0, 12.0, 3.5, -50.0, 50.0);
            pen.arc(13.0, 12.0, 7.5, -50.0, 50.0);
        }
        Icon::Import => {
            pen.path(&[
                (3.0, 15.0),
                (3.0, 19.0),
                (5.0, 21.0),
                (19.0, 21.0),
                (21.0, 19.0),
                (21.0, 15.0),
            ]);
            pen.path(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)]);
            pen.line(12.0, 15.0, 12.0, 3.0);
        }
        Icon::DeliverPage => {
            pen.path(&[
                (3.0, 15.0),
                (3.0, 19.0),
                (5.0, 21.0),
                (19.0, 21.0),
                (21.0, 19.0),
                (21.0, 15.0),
            ]);
            pen.path(&[(7.0, 8.0), (12.0, 3.0), (17.0, 8.0)]);
            pen.line(12.0, 3.0, 12.0, 15.0);
        }
        Icon::Plus => {
            pen.line(5.0, 12.0, 19.0, 12.0);
            pen.line(12.0, 5.0, 12.0, 19.0);
        }
        Icon::Minus => pen.line(5.0, 12.0, 19.0, 12.0),
        Icon::Fit => {
            for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                let (x, y) = (12.0 - 9.0 * sx, 12.0 - 8.0 * sy);
                pen.path(&[(x, y + 4.5 * sy), (x, y), (x + 4.5 * sx, y)]);
            }
            pen.line(8.0, 12.0, 16.0, 12.0);
        }
        Icon::MediaPool => {
            pen.rect(3.0, 3.0, 7.5, 7.5, 1.5);
            pen.rect(13.5, 3.0, 7.5, 7.5, 1.5);
            pen.rect(3.0, 13.5, 7.5, 7.5, 1.5);
            pen.rect(13.5, 13.5, 7.5, 7.5, 1.5);
        }
        Icon::Effects => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = TAU * i as f32 / 10.0 - PI / 2.0;
                    let rad = if i % 2 == 0 { 9.5 } else { 4.2 };
                    (12.0 + a.cos() * rad, 12.8 + a.sin() * rad)
                })
                .collect();
            pen.closed(&pts);
        }
        Icon::Inspector => {
            for (y, x) in [(6.0, 15.0), (12.0, 8.0), (18.0, 16.0)] {
                pen.line(3.0, y, x - 2.5, y);
                pen.line(x + 2.5, y, 21.0, y);
                pen.circle(x, y, 2.5);
            }
        }
        Icon::Insert => {
            pen.rect(2.5, 10.0, 19.0, 9.0, 2.0);
            pen.line(12.0, 10.0, 12.0, 19.0);
            pen.line(12.0, 2.5, 12.0, 7.0);
            pen.head(12.0, 7.0, 0.0, 1.0, 2.6);
        }
        Icon::Overwrite => {
            pen.rect(2.5, 10.0, 19.0, 9.0, 2.0);
            pen.frect(8.0, 10.0, 8.0, 9.0, 0.5);
            pen.line(12.0, 2.5, 12.0, 7.0);
            pen.head(12.0, 7.0, 0.0, 1.0, 2.6);
        }
        Icon::Append => {
            pen.rect(2.5, 10.0, 13.0, 9.0, 2.0);
            pen.frect(17.5, 10.0, 4.0, 9.0, 1.5);
            pen.line(19.5, 2.5, 19.5, 7.0);
            pen.head(19.5, 7.0, 0.0, 1.0, 2.6);
        }
        Icon::Skim => {
            pen.rect(3.0, 7.0, 18.0, 12.0, 2.0);
            pen.line(12.0, 4.0, 12.0, 21.5);
            pen.fill(&[(9.0, 2.5), (15.0, 2.5), (12.0, 5.8)]);
        }
        Icon::Trim => {
            pen.path(&[(5.0, 4.0), (10.0, 4.0), (10.0, 20.0), (5.0, 20.0)]);
            pen.path(&[(19.0, 4.0), (14.0, 4.0), (14.0, 20.0), (19.0, 20.0)]);
        }
        Icon::Envelope => {
            pen.path(&[(3.0, 17.0), (9.0, 8.0), (15.0, 14.0), (21.0, 6.0)]);
            pen.disc(9.0, 8.0, 2.0);
            pen.disc(15.0, 14.0, 2.0);
        }
        Icon::IBeam => {
            pen.path(&[
                (8.0, 3.5),
                (10.0, 3.5),
                (12.0, 5.5),
                (14.0, 3.5),
                (16.0, 3.5),
            ]);
            pen.line(12.0, 5.5, 12.0, 18.5);
            pen.path(&[
                (8.0, 20.5),
                (10.0, 20.5),
                (12.0, 18.5),
                (14.0, 20.5),
                (16.0, 20.5),
            ]);
        }
        Icon::Keyframe => pen.closed(&[(12.0, 4.0), (20.0, 12.0), (12.0, 20.0), (4.0, 12.0)]),
        Icon::Reset => {
            pen.arc(12.0, 12.0, 8.0, 200.0, 520.0);
            let a = 200.0f32.to_radians();
            let (x, y) = (12.0 + 8.0 * a.cos(), 12.0 + 8.0 * a.sin());
            pen.head(x, y, a.sin(), -a.cos(), 3.0);
        }
        // ---------------------------------------------------------------- pintura
        Icon::Brush => {
            pen.closed(&[(9.5, 12.5), (17.5, 4.0), (20.0, 6.5), (11.5, 14.5)]);
            pen.closed(&[
                (8.0, 14.5),
                (5.2, 16.0),
                (4.5, 19.0),
                (2.5, 21.0),
                (7.0, 21.0),
                (9.8, 19.5),
                (10.8, 17.2),
            ]);
        }
        Icon::Eraser => {
            pen.closed(&[
                (3.0, 15.0),
                (13.0, 5.0),
                (20.0, 12.0),
                (11.0, 21.0),
                (8.5, 21.0),
            ]);
            pen.line(7.5, 10.5, 15.0, 18.0);
            pen.line(11.0, 21.0, 21.0, 21.0);
        }
        Icon::Smudge => {
            pen.line(3.0, 8.0, 12.0, 8.0);
            pen.line(3.0, 12.0, 10.0, 12.0);
            pen.line(3.0, 16.0, 12.0, 16.0);
            pen.circle(16.5, 12.0, 4.5);
        }
        Icon::Clone => {
            // carimbo: cabo, base larga e o "pé" com a marca
            pen.rect(9.0, 3.0, 6.0, 9.0, 3.0);
            pen.line(5.0, 12.0, 19.0, 12.0);
            pen.rect(5.0, 12.0, 14.0, 4.0, 1.5);
            pen.line(4.0, 21.0, 20.0, 21.0);
            pen.circle(12.0, 18.5, 1.5);
        }
        Icon::Bucket => {
            pen.closed(&[(11.0, 3.5), (19.0, 11.5), (11.5, 19.0), (3.5, 11.0)]);
            pen.line(5.0, 13.0, 17.5, 13.0);
            pen.line(5.0, 2.0, 9.0, 6.0);
            pen.fill(&[(20.5, 15.0), (18.6, 19.0), (22.4, 19.0)]);
            pen.disc(20.5, 19.6, 1.9);
        }
        Icon::Gradient => {
            pen.rect(3.0, 4.0, 18.0, 16.0, 2.0);
            for i in 0..4 {
                let x = 4.0 + i as f32 * 4.0;
                let r = Rect::from_min_max(pen.pt(x + 0.2, 5.0), pen.pt(x + 4.0, 19.0));
                painter.rect_filled(r, 0.0, soft(0.15 + 0.25 * i as f32));
            }
        }
        Icon::Eyedropper => {
            pen.path(&[(3.0, 21.0), (3.0, 18.0), (12.0, 9.0)]);
            pen.path(&[(3.0, 21.0), (6.0, 21.0), (15.0, 12.0)]);
            pen.line(10.5, 7.5, 16.5, 13.5);
            pen.closed(&[
                (13.5, 10.5),
                (18.0, 6.0),
                (19.5, 4.5),
                (21.0, 5.0),
                (21.0, 6.5),
                (16.5, 13.5),
            ]);
        }
        Icon::SelRect => {
            pen.dashed(&[
                (3.5, 5.0),
                (20.5, 5.0),
                (20.5, 19.0),
                (3.5, 19.0),
                (3.5, 5.0),
            ]);
        }
        Icon::SelEllipse => {
            pen.dashed(&pen.arc_pts(12.0, 12.0, 9.0, 7.0, 0.0, 360.0));
        }
        Icon::Lasso => {
            pen.dashed(&pen.arc_pts(12.5, 10.0, 9.0, 6.5, 140.0, 470.0));
            pen.circle(6.0, 15.5, 2.0);
            pen.path(&[(6.0, 17.5), (6.8, 20.0), (8.5, 21.5)]);
        }
        Icon::Wand => {
            pen.line(3.0, 21.0, 13.0, 11.0);
            for (x1, y1, x2, y2) in [
                (16.0, 2.5, 16.0, 4.5),
                (16.0, 12.5, 16.0, 14.5),
                (9.5, 8.5, 11.5, 8.5),
                (20.5, 8.5, 22.5, 8.5),
                (18.6, 5.9, 19.8, 4.7),
                (18.6, 11.1, 19.8, 12.3),
                (13.4, 5.9, 12.2, 4.7),
            ] {
                pen.line(x1, y1, x2, y2);
            }
            pen.disc(16.0, 8.5, 1.2);
        }
        Icon::Move => {
            pen.line(12.0, 2.5, 12.0, 21.5);
            pen.line(2.5, 12.0, 21.5, 12.0);
            pen.head(12.0, 2.5, 0.0, -1.0, 3.0);
            pen.head(12.0, 21.5, 0.0, 1.0, 3.0);
            pen.head(2.5, 12.0, -1.0, 0.0, 3.0);
            pen.head(21.5, 12.0, 1.0, 0.0, 3.0);
        }
        Icon::Transform => {
            pen.rect(5.0, 8.0, 14.0, 12.0, 0.0);
            for (x, y) in [(5.0, 8.0), (19.0, 8.0), (5.0, 20.0), (19.0, 20.0)] {
                pen.frect(x - 1.8, y - 1.8, 3.6, 3.6, 0.6);
            }
            pen.line(12.0, 8.0, 12.0, 4.5);
            pen.circle(12.0, 3.0, 1.5);
        }
        Icon::Crop => {
            pen.path(&[(6.0, 2.5), (6.0, 18.0), (21.5, 18.0)]);
            pen.path(&[(2.5, 6.0), (18.0, 6.0), (18.0, 21.5)]);
        }
        Icon::Line => {
            pen.line(5.0, 19.0, 19.0, 5.0);
            pen.disc(5.0, 19.0, 2.0);
            pen.disc(19.0, 5.0, 2.0);
        }
        Icon::Rectangle => pen.rect(3.0, 5.0, 18.0, 14.0, 2.0),
        Icon::Ellipse => pen.ellipse(12.0, 12.0, 9.0, 7.0),
        Icon::Text => {
            pen.path(&[(4.5, 7.0), (4.5, 4.0), (19.5, 4.0), (19.5, 7.0)]);
            pen.line(12.0, 4.0, 12.0, 20.0);
            pen.line(9.0, 20.0, 15.0, 20.0);
        }
        Icon::Layers => {
            pen.closed(&[(12.0, 2.5), (21.5, 7.5), (12.0, 12.5), (2.5, 7.5)]);
            pen.path(&[(2.5, 12.0), (12.0, 17.0), (21.5, 12.0)]);
            pen.path(&[(2.5, 16.5), (12.0, 21.5), (21.5, 16.5)]);
        }
        Icon::FlipH => {
            pen.closed(&[(3.0, 7.0), (8.5, 12.0), (3.0, 17.0)]);
            pen.closed(&[(21.0, 7.0), (15.5, 12.0), (21.0, 17.0)]);
            for y in [2.5, 8.5, 14.5, 20.5] {
                pen.line(12.0, y, 12.0, y + 1.5);
            }
        }
        Icon::FlipV => {
            pen.closed(&[(7.0, 3.0), (12.0, 8.5), (17.0, 3.0)]);
            pen.closed(&[(7.0, 21.0), (12.0, 15.5), (17.0, 21.0)]);
            for x in [2.5, 8.5, 14.5, 20.5] {
                pen.line(x, 12.0, x + 1.5, 12.0);
            }
        }
        Icon::Swap => {
            pen.line(4.0, 7.5, 19.5, 7.5);
            pen.head(4.0, 7.5, -1.0, 0.0, 3.5);
            pen.line(4.5, 16.5, 20.0, 16.5);
            pen.head(20.0, 16.5, 1.0, 0.0, 3.5);
        }
        // ---------------------------------------------------------------- gerais
        Icon::Folder => pen.closed(&[
            (2.5, 5.0),
            (4.0, 3.5),
            (8.5, 3.5),
            (10.5, 6.0),
            (20.0, 6.0),
            (21.5, 7.5),
            (21.5, 18.5),
            (20.0, 20.0),
            (4.0, 20.0),
            (2.5, 18.5),
        ]),
        Icon::FolderOpen => {
            pen.path(&[
                (20.0, 10.0),
                (20.0, 7.5),
                (18.5, 6.0),
                (10.5, 6.0),
                (8.5, 3.5),
                (4.0, 3.5),
                (2.5, 5.0),
                (2.5, 18.5),
                (4.0, 20.0),
            ]);
            pen.closed(&[
                (4.0, 20.0),
                (7.5, 11.5),
                (9.0, 10.0),
                (21.0, 10.0),
                (22.0, 11.5),
                (19.0, 20.0),
            ]);
        }
        Icon::Save => {
            pen.closed(&[
                (15.5, 3.0),
                (5.0, 3.0),
                (3.0, 5.0),
                (3.0, 19.0),
                (5.0, 21.0),
                (19.0, 21.0),
                (21.0, 19.0),
                (21.0, 8.5),
            ]);
            pen.path(&[(17.0, 21.0), (17.0, 14.0), (7.0, 14.0), (7.0, 21.0)]);
            pen.path(&[(7.0, 3.0), (7.0, 8.0), (14.0, 8.0)]);
        }
        Icon::Duplicate | Icon::Copy => {
            pen.rect(8.0, 8.0, 13.5, 13.5, 2.0);
            pen.path(&[
                (4.5, 16.0),
                (4.0, 16.0),
                (2.5, 14.5),
                (2.5, 4.0),
                (4.0, 2.5),
                (14.5, 2.5),
                (16.0, 4.0),
                (16.0, 4.5),
            ]);
            if icon == Icon::Duplicate {
                pen.line(14.75, 11.5, 14.75, 18.0);
                pen.line(11.5, 14.75, 18.0, 14.75);
            }
        }
        Icon::Paste => {
            pen.rect(8.0, 2.0, 8.0, 4.0, 1.0);
            pen.path(&[
                (16.0, 4.0),
                (18.0, 4.0),
                (20.0, 6.0),
                (20.0, 20.0),
                (18.0, 22.0),
                (6.0, 22.0),
                (4.0, 20.0),
                (4.0, 6.0),
                (6.0, 4.0),
                (8.0, 4.0),
            ]);
        }
        Icon::Trash => {
            pen.line(3.0, 6.0, 21.0, 6.0);
            pen.path(&[
                (5.5, 6.0),
                (6.5, 19.5),
                (8.0, 21.0),
                (16.0, 21.0),
                (17.5, 19.5),
                (18.5, 6.0),
            ]);
            pen.path(&[
                (8.5, 6.0),
                (8.5, 4.0),
                (9.5, 3.0),
                (14.5, 3.0),
                (15.5, 4.0),
                (15.5, 6.0),
            ]);
            pen.line(10.0, 10.5, 10.0, 16.5);
            pen.line(14.0, 10.5, 14.0, 16.5);
        }
        Icon::Close => {
            pen.line(6.0, 6.0, 18.0, 18.0);
            pen.line(18.0, 6.0, 6.0, 18.0);
        }
        Icon::Check => pen.path(&[(4.5, 12.5), (9.5, 17.5), (19.5, 6.5)]),
        Icon::MergeDown => {
            pen.line(12.0, 3.0, 12.0, 16.0);
            pen.path(&[(6.5, 10.5), (12.0, 16.0), (17.5, 10.5)]);
            pen.line(5.0, 20.5, 19.0, 20.5);
        }
        Icon::ChevronUp => pen.path(&[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)]),
        Icon::ChevronDown => pen.path(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Icon::Undo | Icon::Redo => {
            let m = |x: f32| if icon == Icon::Undo { x } else { 24.0 - x };
            pen.path(&[(m(9.0), 14.0), (m(4.0), 9.0), (m(9.0), 4.0)]);
            let mut v = vec![(m(4.0), 9.0), (m(14.5), 9.0)];
            for (x, y) in pen.arc_pts(14.5, 14.5, 5.5, 5.5, -90.0, 90.0) {
                v.push((m(x), y));
            }
            v.push((m(11.0), 20.0));
            pen.path(&v);
        }
        Icon::Scissors => {
            pen.circle(6.0, 6.0, 3.0);
            pen.circle(6.0, 18.0, 3.0);
            pen.line(20.0, 4.0, 8.2, 15.8);
            pen.line(14.5, 14.5, 20.0, 20.0);
            pen.line(8.2, 8.2, 11.5, 11.5);
        }
        Icon::ZoomIn | Icon::ZoomOut | Icon::Search => {
            pen.circle(10.5, 10.5, 7.0);
            pen.line(15.5, 15.5, 21.0, 21.0);
            if icon != Icon::Search {
                pen.line(7.5, 10.5, 13.5, 10.5);
            }
            if icon == Icon::ZoomIn {
                pen.line(10.5, 7.5, 10.5, 13.5);
            }
        }
        Icon::MarkIn | Icon::MarkOut => {
            // colchete com uma setinha apontando para dentro
            let (x, dir) = if icon == Icon::MarkIn {
                (7.0, 1.0)
            } else {
                (17.0, -1.0)
            };
            pen.line(x, 4.0, x, 20.0);
            pen.line(x, 4.0, x + 4.0 * dir, 4.0);
            pen.line(x, 20.0, x + 4.0 * dir, 20.0);
            pen.line(x + 3.0 * dir, 12.0, x + 9.0 * dir, 12.0);
            pen.line(x + 6.5 * dir, 9.0, x + 9.0 * dir, 12.0);
            pen.line(x + 6.5 * dir, 15.0, x + 9.0 * dir, 12.0);
        }
        Icon::Gear => {
            let pts: Vec<(f32, f32)> = (0..32)
                .map(|i| {
                    // 8 dentes: cada um ocupa 4 pontos (sobe, topo, topo, desce).
                    let a = TAU * (i as f32 - 0.5) / 32.0;
                    let rad = if (i % 4) == 1 || (i % 4) == 2 {
                        9.5
                    } else {
                        7.0
                    };
                    (12.0 + a.cos() * rad, 12.0 + a.sin() * rad)
                })
                .collect();
            pen.closed(&pts);
            pen.circle(12.0, 12.0, 3.0);
        }
        Icon::Eye | Icon::EyeOff => {
            // Amêndoa: dois arcos que se encontram em (2, 12) e (22, 12).
            let mut top = pen.arc_pts(12.0, 15.64, 10.64, 10.64, 200.0, 340.0);
            let bottom: Vec<(f32, f32)> = top.iter().rev().map(|&(x, y)| (x, 24.0 - y)).collect();
            top.extend(bottom);
            pen.closed(&top);
            pen.circle(12.0, 12.0, 3.0);
            if icon == Icon::EyeOff {
                pen.line(3.0, 3.0, 21.0, 21.0);
            }
        }
        Icon::Mic => {
            pen.rect(9.0, 2.5, 6.0, 12.0, 3.0);
            pen.arc(12.0, 10.5, 7.0, 0.0, 180.0);
            pen.line(12.0, 17.5, 12.0, 21.5);
        }
        Icon::Sparkles => {
            let star: Vec<(f32, f32)> = (0..8)
                .map(|i| {
                    let a = TAU * i as f32 / 8.0 - PI / 2.0;
                    let rad = if i % 2 == 0 { 8.5 } else { 2.6 };
                    (10.5 + a.cos() * rad, 13.0 + a.sin() * rad)
                })
                .collect();
            pen.closed(&star);
            pen.line(19.0, 2.5, 19.0, 7.5);
            pen.line(16.5, 5.0, 21.5, 5.0);
        }
        Icon::Waveform => {
            for (x, h) in [
                (3.0, 2.0),
                (7.0, 6.0),
                (11.0, 9.0),
                (15.0, 5.0),
                (19.0, 7.5),
            ] {
                pen.line(x, 12.0 - h, x, 12.0 + h);
            }
            pen.line(22.0, 11.0, 22.0, 13.0);
        }
        Icon::Document => {
            pen.closed(&[
                (14.5, 2.5),
                (6.0, 2.5),
                (4.0, 4.5),
                (4.0, 19.5),
                (6.0, 21.5),
                (18.0, 21.5),
                (20.0, 19.5),
                (20.0, 8.0),
            ]);
            pen.path(&[(14.0, 2.5), (14.0, 8.0), (20.0, 8.0)]);
            pen.line(8.0, 13.0, 16.0, 13.0);
            pen.line(8.0, 17.0, 16.0, 17.0);
            pen.line(8.0, 9.0, 10.0, 9.0);
        }
        Icon::Book => {
            pen.closed(&[
                (2.5, 4.0),
                (8.0, 4.0),
                (12.0, 7.0),
                (12.0, 20.5),
                (8.5, 18.0),
                (2.5, 18.0),
            ]);
            pen.closed(&[
                (21.5, 4.0),
                (16.0, 4.0),
                (12.0, 7.0),
                (12.0, 20.5),
                (15.5, 18.0),
                (21.5, 18.0),
            ]);
        }
        Icon::Outline => {
            pen.line(3.0, 5.0, 14.0, 5.0);
            pen.line(7.0, 10.0, 17.0, 10.0);
            pen.line(7.0, 15.0, 15.0, 15.0);
            pen.line(3.0, 20.0, 13.0, 20.0);
            pen.disc(20.0, 5.0, 1.3);
            pen.disc(21.0, 10.0, 1.3);
        }
        Icon::Wheel => {
            pen.circle(12.0, 12.0, 9.0);
            pen.circle(12.0, 12.0, 3.0);
            for a in [45.0f32, 135.0, 225.0, 315.0] {
                let r = a.to_radians();
                pen.line(
                    12.0 + 3.0 * r.cos(),
                    12.0 + 3.0 * r.sin(),
                    12.0 + 9.0 * r.cos(),
                    12.0 + 9.0 * r.sin(),
                );
            }
        }
    }
}

/// Botão só com ícone. `on`: estado ligado (ícone e fundo em destaque).
pub fn button(ui: &mut Ui, icon: Icon, size: f32, on: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size + 8.0, size + 8.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
    let bg = if on {
        Some(theme::ACCENT.gamma_multiply(0.22))
    } else if resp.is_pointer_button_down_on() {
        Some(theme::BAR_LIGHT.gamma_multiply(1.3))
    } else if resp.hovered() {
        Some(theme::BAR_LIGHT)
    } else {
        None
    };
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, 4.0, bg);
    }
    let color = if !ui.is_enabled() {
        theme::TEXT_DIM.gamma_multiply(0.6)
    } else if on {
        theme::ACCENT
    } else if resp.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT
    };
    paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(size, size)),
        icon,
        color,
    );
    if tip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tip)
    }
}

/// Folha com todos os ícones (para conferir o estilo; usada pelo exemplo `icon_sheet`).
pub fn sheet(ui: &mut Ui, size: f32) {
    ui.horizontal_wrapped(|ui| {
        for icon in Icon::ALL {
            let (r, resp) = ui.allocate_exact_size(vec2(size + 12.0, size + 12.0), Sense::hover());
            ui.painter().rect_filled(r, 4.0, theme::BAR);
            paint(ui.painter(), r.shrink(6.0), *icon, theme::TEXT);
            resp.on_hover_text(format!("{icon:?}"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn every_icon_paints_inside_its_square_at_several_sizes() {
        let ctx = egui::Context::default();
        for size in [12.0, 16.0, 24.0, 48.0] {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let r = Rect::from_min_size(pos2(100.0, 100.0), vec2(size, size));
                for icon in Icon::ALL {
                    paint(ui.painter(), r, *icon, Color32::WHITE);
                }
            });
            let mut bounds = Rect::NOTHING;
            for s in &out.shapes {
                bounds = bounds.union(s.shape.visual_bounding_rect());
            }
            // Margem de meio traço para as pontas redondas.
            let allowed =
                Rect::from_min_size(pos2(100.0, 100.0), vec2(size, size)).expand(size * 0.1);
            assert!(allowed.contains_rect(bounds), "{size}: {bounds:?}");
            out.textures_delta.clear();
        }
    }
}
