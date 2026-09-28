use std::{fs::File, path::Path};

use ico::{IconDir, IconDirEntry};
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg::{Options, Tree, TreeParsing},
};
#[cfg(windows)]
use winres::WindowsResource;

fn write_icon(s: u32, tree: &Tree, icon_dir: &mut IconDir) {
    let mut pixmap = Pixmap::new(s, s).unwrap();

    resvg::render(
        tree,
        resvg::FitTo::Size(s, s),
        Transform::default(),
        pixmap.as_mut(),
    )
    .unwrap();

    if s == 256 {
        std::fs::write(
            Path::new(std::env::var_os("OUT_DIR").as_ref().unwrap()).join("icon_256.bitmap"),
            pixmap.data(),
        )
        .unwrap();
    }

    let image = ico::IconImage::from_rgba_data(s, s, pixmap.take());
    icon_dir.add_entry(IconDirEntry::encode(&image).unwrap());
}

/// UI icons: (name, largest size in points). Each is rendered with a full chain of
/// exactly-rasterized mip levels at 4x that size, so it stays sharp up to 400% scaling
/// and when minified, without shipping an SVG renderer in the binary.
const UI_ICONS: [(&str, u32); 9] = [
    ("folder", 26),
    ("stop", 26),
    ("play", 26),
    ("pause", 26),
    ("options", 26),
    ("pin", 26),
    ("error", 64),
    ("warning", 64),
    ("logo", 84),
];

fn write_ui_icons(out_dir: &Path) {
    let mut code = String::new();
    for (name, points) in UI_ICONS {
        let svg_path = format!("assets/{name}.svg");
        println!("cargo:rerun-if-changed={svg_path}");
        let svg = std::fs::read_to_string(&svg_path).unwrap();
        let tree = Tree::from_str(&svg, &Options::default()).unwrap();

        let mut levels = Vec::new();
        // Vulkan mip level sizes: max(1, base >> level)
        let mut size = points * 4;
        while size >= 4 {
            let mut pixmap = Pixmap::new(size, size).unwrap();
            resvg::render(&tree, resvg::FitTo::Size(size, size), Transform::default(), pixmap.as_mut())
                .unwrap();
            // Premultiplied RGBA is stored as-is, it's what the texture expects
            let path = out_dir.join(format!("icon_{name}_{}.png", levels.len()));
            ico::IconImage::from_rgba_data(size, size, pixmap.take())
                .write_png(File::create(&path).unwrap())
                .unwrap();
            levels.push(format!("include_bytes!(r\"{}\")", path.display()));
            size /= 2;
        }
        code += &format!(
            "pub const {}: (u32, &[&[u8]]) = ({}, &[{}]);
",
            name.to_uppercase(),
            points * 4,
            levels.join(", ")
        );
    }
    std::fs::write(out_dir.join("ui_icons.rs"), code).unwrap();
}

fn main() {
    println!("cargo:rerun-if-changed=assets/logo.svg");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    write_ui_icons(Path::new(&out_dir));
    let svg = std::fs::read_to_string("assets/logo.svg").unwrap();
    let tree = Tree::from_str(&svg, &Options::default()).unwrap();

    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);

    for s in [16, 24, 32, 48, 96, 128, 256] {
        write_icon(s, &tree, &mut icon_dir);
    }
    let icon_path = Path::new(&out_dir).join("icon.ico");

    icon_dir.write(File::create(&icon_path).unwrap()).unwrap();
    #[cfg(windows)]
    {
        let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();

        if target_env == "msvc" {
            WindowsResource::new()
                .set_icon(icon_path.to_str().unwrap())
                .compile()
                .unwrap();
        } else {
            let rc_path = format!("{out_dir}/icon.rc");
            let res_path = format!("{out_dir}/icon.res.o");
            std::fs::write(&rc_path, format!("1 ICON \"{}\"\n", icon_path.display().to_string().replace('\\', "/")))
                .unwrap();
            let status = std::process::Command::new("windres")
                .args([&rc_path, "-o", &res_path])
                .status()
                .expect("windres not found");
            assert!(status.success(), "windres failed");
            println!("cargo:rustc-link-arg={res_path}");
        }
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows",
        target_os = "macos"
    ))]
    println!("cargo:rustc-cfg=supported_os");
    println!("cargo::rustc-check-cfg=cfg(supported_os)");
}
