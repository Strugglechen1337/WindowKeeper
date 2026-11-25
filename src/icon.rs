//! Procedurally generated tray icon (no external asset needed).

/// Icon edge length in pixels.
pub const SIZE: u32 = 32;

/// RGBA pixels of a blue rounded square showing a stylised window.
pub fn rgba() -> Vec<u8> {
    let n = SIZE as i32;
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    let radius = 6;
    for y in 0..n {
        for x in 0..n {
            // Rounded corners.
            let cx = if x < radius {
                radius - x
            } else if x >= n - radius {
                x - (n - radius - 1)
            } else {
                0
            };
            let cy = if y < radius {
                radius - y
            } else if y >= n - radius {
                y - (n - radius - 1)
            } else {
                0
            };
            if cx * cx + cy * cy > radius * radius {
                continue;
            }
            let (r, g, b) = if (6..=25).contains(&x) && (8..=25).contains(&y) {
                if y <= 11 {
                    (38, 99, 190) // title bar
                } else {
                    (245, 248, 252) // window body
                }
            } else {
                (52, 120, 220) // background
            };
            let i = ((y * n + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_expected_size_and_visible_pixels() {
        let p = rgba();
        assert_eq!(p.len(), (SIZE * SIZE * 4) as usize);
        assert!(p.chunks(4).filter(|c| c[3] == 255).count() > 500);
        // Transparent corner.
        assert_eq!(p[3], 0);
    }
}
