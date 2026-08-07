pub mod manager;
pub mod generator;
pub mod theme;

pub use manager::BrandingManager;
pub use generator::{generate_logo_svg, generate_favicon_svg, generate_favicon_png};
pub use theme::generate_css_variables;
