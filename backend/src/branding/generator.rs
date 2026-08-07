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

pub fn generate_favicon_png(_config: &BrandingConfig) -> Vec<u8> {
    vec![
        137, 80, 78, 71, 13, 10, 26, 10,
        0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
}
