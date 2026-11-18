//! Rasterises SVG documents into sprite bytes.

use resvg::{tiny_skia, usvg};

/// Premultiplied RGBA bytes of `svg` rendered at `size` x `size`.
pub fn rgba(svg: &str, size: u32) -> Result<Vec<u8>, String> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())
        .map_err(|e| format!("parse svg ({size}px): {e}"))?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("zero-sized pixmap")?;
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    Ok(pixmap.data().to_vec())
}

/// Alpha channel of `svg` rendered at `size` x `size`.
pub fn alpha(svg: &str, size: u32) -> Result<Vec<u8>, String> {
    Ok(rgba(svg, size)?.chunks_exact(4).map(|px| px[3]).collect())
}
