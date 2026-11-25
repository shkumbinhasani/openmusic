use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const SIPS: &str = "/usr/bin/sips";
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MIN_SIDE: u32 = 300;
const MAX_SIDE: u32 = 10_000;
const UPLOAD_SIDE: u32 = 2000;
const TYPES: [&str; 3] = ["jpg", "jpeg", "png"];

pub fn check_type(path: &Path) -> Result<(), String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if TYPES.contains(&extension.as_str()) {
        Ok(())
    } else {
        Err(
            "The file you selected isn't supported. Please choose a JPG or PNG file instead."
                .into(),
        )
    }
}

pub fn check_size(bytes: u64) -> Result<(), String> {
    if bytes > MAX_FILE_BYTES {
        Err("Image too large. Please select an image below 10MB.".into())
    } else {
        Ok(())
    }
}

pub fn check_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width < MIN_SIDE || height < MIN_SIDE {
        return Err(format!(
            "Image too small. Images must be at least {MIN_SIDE}x{MIN_SIDE}."
        ));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!(
            "Image too big. Size must be less than {MAX_SIDE} by {MAX_SIDE} pixels."
        ));
    }
    Ok(())
}

pub fn parse_dimensions(output: &str) -> Option<(u32, u32)> {
    let value = |key: &str| {
        output.lines().find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .and_then(|rest| rest.trim().parse::<u32>().ok())
        })
    };
    Some((value("pixelWidth:")?, value("pixelHeight:")?))
}

pub fn needs_downscale(width: u32, height: u32) -> bool {
    width.max(height) > UPLOAD_SIDE
}

fn unreadable() -> String {
    "OpenMusic can’t open this image. Please choose a JPG or PNG file instead.".into()
}

fn scratch_file() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    std::env::temp_dir().join(format!(
        "openmusic-cover-{}-{stamp}.jpg",
        std::process::id()
    ))
}

pub fn cover_jpeg(path: &Path) -> Result<Vec<u8>, String> {
    check_type(path)?;
    let size = std::fs::metadata(path).map_err(|_| unreadable())?.len();
    check_size(size)?;
    let probe = Command::new(SIPS)
        .args(["-g", "pixelWidth", "-g", "pixelHeight"])
        .arg(path)
        .output()
        .map_err(|_| unreadable())?;
    let (width, height) =
        parse_dimensions(&String::from_utf8_lossy(&probe.stdout)).ok_or_else(unreadable)?;
    check_dimensions(width, height)?;
    let out = scratch_file();
    let mut convert = Command::new(SIPS);
    convert.args(["-s", "format", "jpeg", "-s", "formatOptions", "90"]);
    if needs_downscale(width, height) {
        convert.args(["-Z", &UPLOAD_SIDE.to_string()]);
    }
    let converted = convert
        .arg(path)
        .arg("--out")
        .arg(&out)
        .output()
        .map_err(|_| unreadable())?;
    let bytes = std::fs::read(&out);
    let _ = std::fs::remove_file(&out);
    match bytes {
        Ok(bytes) if converted.status.success() && bytes.starts_with(&[0xff, 0xd8]) => Ok(bytes),
        _ => Err(unreadable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_desktop_image_types() {
        assert!(check_type(Path::new("/tmp/cover.JPG")).is_ok());
        assert!(check_type(Path::new("cover.png")).is_ok());
        assert!(check_type(Path::new("cover.jpeg")).is_ok());
        assert!(check_type(Path::new("cover.heic")).is_err());
        assert!(check_type(Path::new("cover")).is_err());
    }

    #[test]
    fn applies_the_desktop_size_rules() {
        assert!(check_size(10 * 1024 * 1024).is_ok());
        assert!(check_size(10 * 1024 * 1024 + 1).is_err());
        assert!(check_dimensions(300, 300).is_ok());
        assert_eq!(
            check_dimensions(299, 640).unwrap_err(),
            "Image too small. Images must be at least 300x300."
        );
        assert!(check_dimensions(10_000, 10_000).is_ok());
        assert!(check_dimensions(10_001, 400).is_err());
        assert!(!needs_downscale(2000, 1500));
        assert!(needs_downscale(2001, 300));
    }

    #[test]
    fn reads_sips_dimensions() {
        let output = "/tmp/cover.png\n  pixelWidth: 640\n  pixelHeight: 480\n";
        assert_eq!(parse_dimensions(output), Some((640, 480)));
        assert_eq!(parse_dimensions("/tmp/x\n  pixelWidth: 640\n"), None);
        assert_eq!(parse_dimensions(""), None);
    }

    #[test]
    fn converts_a_png_to_jpeg() {
        if !Path::new(SIPS).exists() {
            return;
        }
        let directory =
            std::env::temp_dir().join(format!("openmusic-image-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let tiny = directory.join("tiny.png");
        std::fs::write(&tiny, PNG_1X1).unwrap();
        assert!(
            cover_jpeg(&tiny)
                .unwrap_err()
                .starts_with("Image too small")
        );
        let big = directory.join("big.png");
        let status = Command::new(SIPS)
            .args(["-z", "2400", "2400"])
            .arg(&tiny)
            .arg("--out")
            .arg(&big)
            .output()
            .unwrap();
        assert!(status.status.success());
        let jpeg = cover_jpeg(&big).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8]));
        let check = directory.join("check.jpg");
        std::fs::write(&check, &jpeg).unwrap();
        let probe = Command::new(SIPS)
            .args(["-g", "pixelWidth", "-g", "pixelHeight"])
            .arg(&check)
            .output()
            .unwrap();
        assert_eq!(
            parse_dimensions(&String::from_utf8_lossy(&probe.stdout)),
            Some((2000, 2000))
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
}
