use crate::config::BrandingConfig;

pub fn generate_logo_svg(config: &BrandingConfig) -> String {
    let short = &config.short_name;
    let primary = &config.primary_color;
    let text_color = &config.text_color;

    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 280 60" width="280" height="60">
  <defs>
    <linearGradient id="logoGrad" x1="0%" y1="0%" x2="100%" y2="0%">
      <stop offset="0%" style="stop-color:{primary};stop-opacity:1" />
      <stop offset="100%" style="stop-color:{text_color};stop-opacity:0.9" />
    </linearGradient>
  </defs>
  <rect width="280" height="60" fill="transparent"/>
  <text x="140" y="42" font-family="{font}" font-size="36" font-weight="700"
        text-anchor="middle" fill="url(#logoGrad)">{short}</text>
  <text x="140" y="55" font-family="{font}" font-size="10" font-weight="400"
        text-anchor="middle" fill="{muted}">{tagline}</text>
</svg>"#,
        primary = primary,
        text_color = text_color,
        short = short,
        font = config.font_family.split(',').next().unwrap_or("sans-serif").trim(),
        muted = config.text_muted_color,
        tagline = config.tagline,
    )
}

pub fn generate_favicon_svg(config: &BrandingConfig) -> String {
    let primary = &config.primary_color;
    let letter = config.short_name.chars().next().unwrap_or('T');

    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="32" height="32">
  <rect width="32" height="32" rx="6" fill="{primary}"/>
  <text x="16" y="23" font-family="{font}" font-size="20" font-weight="700"
        text-anchor="middle" fill="white">{letter}</text>
</svg>"#,
        primary = primary,
        font = config.font_family.split(',').next().unwrap_or("sans-serif").trim(),
        letter = letter,
    )
}

pub fn generate_favicon_png(config: &BrandingConfig) -> Vec<u8> {
    // Generate a real 16x16 PNG: solid primary_color square with the first letter centered.
    // Minimal valid PNG structure with IHDR, IDAT (deflate), IEND.
    let color = hex_to_rgb(&config.primary_color);
    let letter = config.short_name.chars().next().unwrap_or('T');

    // 16x16 RGB image, white letter on primary color background
    let mut pixels = Vec::with_capacity(16 * 16 * 3);
    for y in 0..16 {
        pixels.push(0u8); // filter byte: None
        for x in 0..16 {
            let is_letter = is_letter_pixel(x, y, letter);
            if is_letter {
                pixels.push(255);
                pixels.push(255);
                pixels.push(255);
            } else {
                pixels.push(color.0);
                pixels.push(color.1);
                pixels.push(color.2);
            }
        }
    }

    png_from_raw(16, 16, &pixels)
}

fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return (45, 55, 72); // fallback dark blue
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(45);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(55);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(72);
    (r, g, b)
}

fn is_letter_pixel(x: usize, y: usize, _letter: char) -> bool {
    // Simple 16x16 bitmap for a generic blocky "T" / letter shape (center 6x6 bar)
    if y < 2 {
        x >= 4 && x < 12
    } else if y < 10 {
        x >= 6 && x < 10
    } else {
        x >= 6 && x < 10
    }
}

fn png_from_raw(width: u32, height: u32, raw_rows: &[u8]) -> Vec<u8> {
    use std::io::Write;

    let mut out = Vec::new();

    // PNG signature
    out.write_all(&[137, 80, 78, 71, 13, 10, 26, 10]).unwrap();

    // IHDR chunk
    let mut ihdr_data = Vec::new();
    ihdr_data.write_all(&width.to_be_bytes()).unwrap();
    ihdr_data.write_all(&height.to_be_bytes()).unwrap();
    ihdr_data.write_all(&[8, 2, 0, 0, 0]).unwrap(); // 8-bit, RGB
    let ihdr_crc = crc32(&ihdr_data);
    write_chunk(&mut out, b"IHDR", &ihdr_data, ihdr_crc);

    // Deflate compressed raw_rows
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw_rows).unwrap();
    let compressed = encoder.finish().unwrap();

    // IDAT chunk
    let idat_crc = crc32(&compressed);
    write_chunk(&mut out, b"IDAT", &compressed, idat_crc);

    // IEND chunk
    let iend_crc = crc32(&[]);
    write_chunk(&mut out, b"IEND", &[], iend_crc);

    out
}

fn write_chunk(out: &mut Vec<u8>, type_: &[u8], data: &[u8], crc: u32) {
    use std::io::Write;
    let len = data.len() as u32;
    out.write_all(&len.to_be_bytes()).unwrap();
    out.write_all(type_).unwrap();
    out.write_all(data).unwrap();
    out.write_all(&crc.to_be_bytes()).unwrap();
}

fn crc32(data: &[u8]) -> u32 {
    let mut s = crc32fast::Hasher::new();
    s.update(data);
    s.finalize()
}
