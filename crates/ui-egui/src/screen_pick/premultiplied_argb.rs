//! X11: a pointer picture's pixels as the server takes them

/// Straight-alpha RGBA pixels as the premultiplied 32-bit ARGB a pointer picture is sent in, in the server's byte order
pub(crate) fn premultiplied_argb(rgba: &[u8], least_significant_first: bool) -> Vec<u8> {
    let mut data = Vec::with_capacity(rgba.len());
    for &[red, green, blue, alpha] in rgba.as_chunks::<4>().0 {
        let [red, green, blue] = [red, green, blue].map(|value| u8::try_from(u16::from(value) * u16::from(alpha) / 255).unwrap_or(u8::MAX));
        if least_significant_first {
            data.extend_from_slice(&[blue, green, red, alpha]);
        } else {
            data.extend_from_slice(&[alpha, red, green, blue]);
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pointer_pixel_is_sent_premultiplied_in_the_servers_byte_order() {
        let rgba = [255, 128, 0, 128, 10, 20, 30, 0];

        assert_eq!(premultiplied_argb(&rgba, true), vec![0, 64, 128, 128, 0, 0, 0, 0]);
        assert_eq!(premultiplied_argb(&rgba, false), vec![128, 128, 64, 0, 0, 0, 0, 0]);
    }
}
