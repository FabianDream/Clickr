fn render_icon_pixels(size: usize) -> Vec<u32> {
    let mut pixels = vec![0u32; size * size];
    let ss = 4;
    let inv_ss2 = 1.0 / (ss * ss) as f32;

    let s = size as f32;
    let radius = s * 0.24;
    let margin = s * 0.06;
    let b_min = margin;
    let b_max = s - margin;

    let tip_x = s * 0.36;
    let tip_y = s * 0.30;

    for y in 0..size {
        for x in 0..size {
            let mut r_accum = 0.0f32;
            let mut g_accum = 0.0f32;
            let mut b_accum = 0.0f32;
            let mut a_accum = 0.0f32;

            for sy in 0..ss {
                for sx in 0..ss {
                    let px = x as f32 + (sx as f32 + 0.5) / ss as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / ss as f32;

                    let dx = if px < b_min + radius {
                        b_min + radius - px
                    } else if px > b_max - radius {
                        px - (b_max - radius)
                    } else {
                        0.0
                    };

                    let dy = if py < b_min + radius {
                        b_min + radius - py
                    } else if py > b_max - radius {
                        py - (b_max - radius)
                    } else {
                        0.0
                    };

                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist <= radius {
                        let border_width = (s * 0.06).max(1.0);
                        let is_border = dist >= radius - border_width;

                        let (br, bg, bb) = if is_border {
                            (56.0, 189.0, 248.0) // Vibrant cyan
                        } else {
                            let t = py / s;
                            (24.0 + 10.0 * t, 28.0 + 12.0 * t, 38.0 + 18.0 * t)
                        };

                        let mut pr = br;
                        let mut pg = bg;
                        let mut pb = bb;

                        let d_tip = ((px - tip_x).powi(2) + (py - tip_y).powi(2)).sqrt();
                        let r1 = s * 0.22;
                        let r2 = s * 0.38;
                        if (d_tip - r1).abs() < (s * 0.04).max(0.8) && px < tip_x + s * 0.1 {
                            pr = 56.0;
                            pg = 189.0;
                            pb = 248.0;
                        } else if (d_tip - r2).abs() < (s * 0.035).max(0.7) && px < tip_x + s * 0.05 {
                            pr = 14.0;
                            pg = 165.0;
                            pb = 233.0;
                        }

                        let vx = px - tip_x;
                        let vy = py - tip_y;

                        if is_point_in_cursor(vx, vy, s) {
                            pr = 255.0;
                            pg = 255.0;
                            pb = 255.0;
                        } else if is_point_in_cursor(vx - 0.5, vy - 0.5, s) {
                            pr = pr * 0.5;
                            pg = pg * 0.5;
                            pb = pb * 0.5;
                        }

                        r_accum += pr;
                        g_accum += pg;
                        b_accum += pb;
                        a_accum += 255.0;
                    }
                }
            }

            let a = (a_accum * inv_ss2).round() as u32;
            let r = (r_accum * inv_ss2).round() as u32;
            let g = (g_accum * inv_ss2).round() as u32;
            let b = (b_accum * inv_ss2).round() as u32;

            pixels[y * size + x] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }

    pixels
}

fn is_point_in_cursor(vx: f32, vy: f32, s: f32) -> bool {
    let scale = s / 32.0;
    let x = vx / scale;
    let y = vy / scale;

    if x < 0.0 || y < 0.0 || x > 14.0 || y > 16.0 {
        return false;
    }

    let p = [
        (0.0f32, 0.0f32),
        (0.0, 13.0),
        (3.5, 9.5),
        (7.0, 14.5),
        (9.5, 13.0),
        (6.0, 8.0),
        (10.0, 8.0),
    ];

    let mut inside = false;
    let mut j = p.len() - 1;
    for i in 0..p.len() {
        let (xi, yi) = p[i];
        let (xj, yj) = p[j];
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn make_ico_dib(size: usize, pixels: &[u32]) -> Vec<u8> {
    let mut dib = Vec::new();
    let and_row_bytes = ((size + 31) / 32) * 4;
    let and_mask_size = and_row_bytes * size;
    let xor_size = size * size * 4;

    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&(size as i32).to_le_bytes());
    dib.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&((xor_size + and_mask_size) as u32).to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());

    for y in (0..size).rev() {
        for x in 0..size {
            let argb = pixels[y * size + x];
            let a = ((argb >> 24) & 0xFF) as u8;
            let r = ((argb >> 16) & 0xFF) as u8;
            let g = ((argb >> 8) & 0xFF) as u8;
            let b = (argb & 0xFF) as u8;
            dib.push(b);
            dib.push(g);
            dib.push(r);
            dib.push(a);
        }
    }

    for y in (0..size).rev() {
        let mut row_bytes = vec![0u8; and_row_bytes];
        for x in 0..size {
            let argb = pixels[y * size + x];
            let a = (argb >> 24) & 0xFF;
            if a < 128 {
                let byte_idx = x / 8;
                let bit_idx = 7 - (x % 8);
                row_bytes[byte_idx] |= 1 << bit_idx;
            }
        }
        dib.extend_from_slice(&row_bytes);
    }

    dib
}

fn generate_app_ico(path: &str) {
    let sizes = [16usize, 32, 48];
    let mut images_data = Vec::new();

    for &size in &sizes {
        let pixels = render_icon_pixels(size);
        let dib = make_ico_dib(size, &pixels);
        images_data.push((size, dib));
    }

    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());

    let header_size = 6 + 16 * sizes.len();
    let mut current_offset = header_size as u32;

    for (size, dib) in &images_data {
        let w = if *size >= 256 { 0 } else { *size as u8 };
        let h = if *size >= 256 { 0 } else { *size as u8 };
        ico.push(w);
        ico.push(h);
        ico.push(0);
        ico.push(0);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        ico.extend_from_slice(&current_offset.to_le_bytes());
        current_offset += dib.len() as u32;
    }

    for (_, dib) in images_data {
        ico.extend_from_slice(&dib);
    }

    std::fs::write(path, ico).unwrap();
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let out_dir = std::env::var("OUT_DIR").unwrap();

        let ico_path = format!("{}/app.ico", manifest_dir.replace('\\', "/"));
        let manifest_path = format!("{}/app.manifest", manifest_dir.replace('\\', "/"));
        let rc_path = format!("{}/app.rc", out_dir.replace('\\', "/"));
        let obj_path = format!("{}/app.o", out_dir.replace('\\', "/"));

        generate_app_ico(&ico_path);

        // In Windows, application manifest is resource ID 1 of type RT_MANIFEST (24).
        // 1 ICON binds the application icon.
        let rc_content = format!(
            "1 ICON \"{}\"\n1 24 \"{}\"\n",
            ico_path, manifest_path
        );
        std::fs::write(&rc_path, rc_content).unwrap();

        let status = std::process::Command::new("windres")
            .args(&[&rc_path, "-o", &obj_path])
            .status()
            .expect("Failed to execute windres");

        if status.success() {
            println!("cargo:rustc-link-arg={}", obj_path);
        }
    }
}
