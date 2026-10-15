//! Paints the main window off-screen from egui's own triangles, to look at the design in any state
//! without a GPU, the real data folder or real games
//! `DQA_SNAPSHOTS=<folder> cargo test snapshots -- --ignored` writes one .ppm per state

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::epaint::{ImageData, Primitive, TextureId};
use eframe::egui::{Pos2, RawInput, vec2};

use super::info_icons::test_ctx;
use super::queue::{app_in, busy, sandbox};
use super::*;

struct Texture {
    size: [usize; 2],
    pixels: Vec<egui::Color32>,
}

/// Premultiplied, 0 to 1, in the same gamma space egui's colours are
struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(width: usize, height: usize, ground: egui::Color32) -> Self {
        Self {
            width,
            height,
            pixels: vec![rgba(ground); width * height],
        }
    }

    fn save_ppm(&self, path: &Path) {
        let mut out = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        for p in &self.pixels {
            for c in &p[..3] {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                out.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        fs::write(path, out).unwrap();
    }
}

fn rgba(c: egui::Color32) -> [f32; 4] {
    let f = |v: u8| f32::from(v) / 255.0;
    [f(c.r()), f(c.g()), f(c.b()), f(c.a())]
}

fn apply(textures: &mut HashMap<TextureId, Texture>, delta: &egui::TexturesDelta) {
    for (id, changes) in &delta.set {
        for change in changes {
            let ImageData::Color(image) = &change.image;
            match change.pos {
                None => {
                    textures.insert(
                        *id,
                        Texture {
                            size: image.size,
                            pixels: image.pixels.clone(),
                        },
                    );
                }
                Some([x0, y0]) => {
                    let texture = textures.get_mut(id).unwrap();
                    for y in 0..image.size[1] {
                        for x in 0..image.size[0] {
                            texture.pixels[(y0 + y) * texture.size[0] + x0 + x] =
                                image.pixels[y * image.size[0] + x];
                        }
                    }
                }
            }
        }
    }
}

/// Bilinear, edges clamped
fn sample(texture: &Texture, uv: Pos2) -> [f32; 4] {
    #[allow(clippy::cast_precision_loss)]
    let (tex_w, tex_h) = (texture.size[0] as f32, texture.size[1] as f32);
    let along = (uv.x * tex_w - 0.5).clamp(0.0, tex_w - 1.0);
    let down = (uv.y * tex_h - 0.5).clamp(0.0, tex_h - 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (left, top) = (along.floor() as usize, down.floor() as usize);
    let right = (left + 1).min(texture.size[0] - 1);
    let bottom = (top + 1).min(texture.size[1] - 1);
    let (fx, fy) = (along.fract(), down.fract());
    let at = |px: usize, py: usize| rgba(texture.pixels[py * texture.size[0] + px]);
    let (nw, ne, sw, se) = (
        at(left, top),
        at(right, top),
        at(left, bottom),
        at(right, bottom),
    );
    let mut out = [0.0; 4];
    for i in 0..4 {
        let upper = nw[i] + (ne[i] - nw[i]) * fx;
        let lower = sw[i] + (se[i] - sw[i]) * fx;
        out[i] = upper + (lower - upper) * fy;
    }
    out
}

fn paint(
    canvas: &mut Canvas,
    textures: &HashMap<TextureId, Texture>,
    output: egui::FullOutput,
    ctx: &egui::Context,
) {
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    #[allow(clippy::cast_precision_loss)]
    let screen =
        egui::Rect::from_min_size(Pos2::ZERO, vec2(canvas.width as f32, canvas.height as f32));
    for clipped in primitives {
        let Primitive::Mesh(mesh) = clipped.primitive else {
            continue;
        };
        let Some(texture) = textures.get(&mesh.texture_id) else {
            continue;
        };
        let clip = clipped.clip_rect.intersect(screen);
        for tri in mesh.indices.as_chunks::<3>().0 {
            let v = [
                mesh.vertices[tri[0] as usize],
                mesh.vertices[tri[1] as usize],
                mesh.vertices[tri[2] as usize],
            ];
            let area = (v[1].pos - v[0].pos).x * (v[2].pos - v[0].pos).y
                - (v[1].pos - v[0].pos).y * (v[2].pos - v[0].pos).x;
            if area.abs() < 1e-6 {
                continue;
            }
            let min = v
                .iter()
                .fold(Pos2::new(f32::MAX, f32::MAX), |m, p| m.min(p.pos));
            let max = v
                .iter()
                .fold(Pos2::new(f32::MIN, f32::MIN), |m, p| m.max(p.pos));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (x0, y0, x1, y1) = (
                min.x.max(clip.left()).floor() as usize,
                min.y.max(clip.top()).floor() as usize,
                (max.x.min(clip.right()).ceil() as usize).min(canvas.width),
                (max.y.min(clip.bottom()).ceil() as usize).min(canvas.height),
            );
            let colors = v.map(|p| rgba(p.color));
            for y in y0..y1 {
                for x in x0..x1 {
                    #[allow(clippy::cast_precision_loss)]
                    let p = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let edge = |a: Pos2, b: Pos2| (b - a).x * (p - a).y - (b - a).y * (p - a).x;
                    let w = [
                        edge(v[1].pos, v[2].pos) / area,
                        edge(v[2].pos, v[0].pos) / area,
                        edge(v[0].pos, v[1].pos) / area,
                    ];
                    if w.iter().any(|w| *w < 0.0) {
                        continue;
                    }
                    let uv = Pos2::new(
                        w[0] * v[0].uv.x + w[1] * v[1].uv.x + w[2] * v[2].uv.x,
                        w[0] * v[0].uv.y + w[1] * v[1].uv.y + w[2] * v[2].uv.y,
                    );
                    let tex = sample(texture, uv);
                    let mut src = [0.0; 4];
                    for (i, s) in src.iter_mut().enumerate() {
                        let vertex =
                            w[0] * colors[0][i] + w[1] * colors[1][i] + w[2] * colors[2][i];
                        *s = vertex * tex[i];
                    }
                    let dst = &mut canvas.pixels[y * canvas.width + x];
                    for i in 0..4 {
                        dst[i] = src[i] + dst[i] * (1.0 - src[3]);
                    }
                }
            }
        }
    }
}

/// A few frames so layout settles and textures land, then the last one painted
fn render(
    app: &mut QuestApp,
    width: usize,
    height: usize,
    setup: impl Fn(&egui::Context),
) -> Canvas {
    let ctx = test_ctx();
    // Icons are textures of the context that loaded them, and each render has its own
    app.queue_icons = None;
    setup(&ctx);
    let mut textures = HashMap::new();
    let mut last = None;
    for frame in 0..4 {
        #[allow(clippy::cast_precision_loss)]
        let input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                vec2(width as f32, height as f32),
            )),
            time: Some(f64::from(frame) * 0.5),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.status_strip(ui);
            app.body(ui);
        });
        apply(&mut textures, &output.textures_delta);
        output.textures_delta.clear();
        last = Some(output);
    }
    let mut canvas = Canvas::new(width, height, theme::GROUND);
    paint(&mut canvas, &textures, last.unwrap(), &ctx);
    canvas
}

fn catalog() -> Vec<Game> {
    let game = |name: &str, exes: &[&str]| Game {
        name: name.into(),
        aliases: Vec::new(),
        exes: exes.iter().map(|e| (*e).to_owned()).collect(),
        steam: Vec::new(),
    };
    vec![
        game("Dragon's Dogma II", &["dragons dogma 2/dd2.exe"]),
        game("Dragon's Dogma: Dark Arisen", &["ddda.exe"]),
        game(
            "Dragon Quest XI S",
            &["dragon quest xi s/dragon quest xi s.exe"],
        ),
        game(
            "Dragon Ball FighterZ",
            &[
                "redbin/win64/red-win64-shipping.exe",
                "redbin/win64/red.exe",
            ],
        ),
        game(
            "Dragon Age: The Veilguard",
            &["dragon age the veilguard/dragon age the veilguard.exe"],
        ),
        game("Where Winds Meet", &["wwm.exe"]),
        game("Marathon", &["marathon.exe"]),
        game("Muck", &["muck.exe"]),
        game("Toxic Commando", &["toxiccommando.exe"]),
    ]
}

/// Two done, one running, three waiting, and a search for dragon with a waiting game picked
fn evening(tmp: &Path) -> QuestApp {
    let mut app = app_in(tmp);
    app.catalog.games = catalog();
    app.query = "dragon".into();
    app.results = vec![0, 1, 2, 3, 4];
    // Toxic Commando was stopped early, Dark Arisen ran its time
    app.done.push(Done {
        game: "Toxic Commando".into(),
        played: Duration::from_secs(11 * 60 + 48),
        full: false,
    });
    app.done.push(Done {
        game: "Dragon's Dogma: Dark Arisen".into(),
        played: Duration::from_secs(16 * 60),
        full: true,
    });
    let mut run = busy("Dragon's Dogma II");
    run.length = Duration::from_secs(16 * 60);
    run.path = PathBuf::from(
        r"C:\Users\you\AppData\Local\Discord Quest Assistant\data\runs\s1792\dragons dogma 2\dd2.exe",
    );
    app.running.push(run);
    for (name, exe) in [
        ("Where Winds Meet", "wwm.exe"),
        ("Marathon", "marathon.exe"),
        ("Muck", "muck.exe"),
    ] {
        let mut item = sandbox(name);
        item.minutes = 16;
        item.target = Target::Sandbox(ExePath::parse(exe).unwrap());
        app.queue.push(item);
    }
    app.select(5);
    app.status = Status::good(
        "Dragon's Dogma: Dark Arisen has run its full time, and its files are cleaned up.",
    );
    app
}

/// Three games queued and waiting for Start Queue, nothing run yet
fn waiting(tmp: &Path) -> QuestApp {
    let mut app = app_in(tmp);
    app.catalog.games = catalog();
    app.query.clear();
    app.results.clear();
    app.selected = None;
    for (name, exe) in [
        ("Where Winds Meet", "wwm.exe"),
        ("Marathon", "marathon.exe"),
        ("Muck", "muck.exe"),
    ] {
        let mut item = sandbox(name);
        item.minutes = 16;
        item.target = Target::Sandbox(ExePath::parse(exe).unwrap());
        app.queue.push(item);
    }
    app.status = Status::good("Muck is number 3 in the queue.");
    app
}

/// Nothing queued yet, a search for dragon with a two-exe game picked and its Executable ⓘ open
fn setup(tmp: &Path) -> QuestApp {
    let mut app = app_in(tmp);
    app.catalog.games = catalog();
    app.query = "dragon".into();
    app.results = vec![0, 1, 2, 3, 4];
    app.select(3);
    app.status = Status::good("Discord's game list is up to date: 19,266 games.");
    app
}

#[test]
#[ignore = "writes images for a person to look at"]
fn snapshots() {
    let Some(dir) = std::env::var_os("DQA_SNAPSHOTS") else {
        return;
    };
    let dir = PathBuf::from(dir);
    fs::create_dir_all(&dir).unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let mut app = evening(tmp.path());
    for (name, width) in [
        ("evening-1080", 1080),
        ("evening-760", 760),
        ("evening-520", 520),
    ] {
        render(&mut app, width, 700, |_| {}).save_ppm(&dir.join(format!("{name}.ppm")));
    }
    app.stop_all();

    let mut app = waiting(tmp.path());
    render(&mut app, 1080, 700, |_| {}).save_ppm(&dir.join("waiting-1080.ppm"));

    let mut app = setup(tmp.path());
    let open_executable = |ctx: &egui::Context| {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(("info", "Executable")), true));
    };
    render(&mut app, 1080, 700, open_executable).save_ppm(&dir.join("setup-1080.ppm"));
}
