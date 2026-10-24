use packconv_core::{convert, ConvertOptions, Limits};
use rars::{ArchiveVersion, Builder};

fn tiny_png() -> Vec<u8> {
    let mut png = Vec::new();
    image::DynamicImage::new_rgba8(16, 16)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

fn tiny_pack_rar() -> Vec<u8> {
    let mut builder = Builder::new(ArchiveVersion::Rar50).compression_level(Some(3));
    builder
        .add_bytes(
            b"pack.mcmeta".to_vec(),
            br#"{"pack":{"pack_format":3,"description":"rar test"}}"#.to_vec(),
            None,
            None,
        )
        .unwrap();
    builder
        .add_bytes(b"assets/minecraft/textures/blocks/stone.png".to_vec(), tiny_png(), None, None)
        .unwrap();
    builder.to_bytes().unwrap()
}

#[test]
fn converts_rar_pack() {
    let rar = tiny_pack_rar();
    let out = convert(
        &rar,
        &ConvertOptions { from: None, to: "1.16.5".into(), limits: Limits::default() },
    )
    .expect("RAR input is accepted");

    assert_eq!(out.report.from, "1.12.2");
    assert!(out.report.files_in >= 2);

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(out.zip)).unwrap();
    let mut meta = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("pack.mcmeta").unwrap(), &mut meta).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
    assert_eq!(meta["pack"]["pack_format"], 6);
}

#[test]
fn rejects_garbage() {
    let result = convert(
        b"not an archive",
        &ConvertOptions { from: None, to: "1.16.5".into(), limits: Limits::default() },
    );
    assert!(result.is_err());
}
