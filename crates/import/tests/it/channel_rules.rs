//! The channel rule table, measured once in the reference app (a fresh project declaring every channel type it
//! offers, one fill giving each channel its own value, and an older project opened there): per channel type its index
//! in the compact layout, its bit in the older bit-mask layout, TextureCraft's channel, whether uniform values are
//! stored as linear colours, and what it shows where no layer covers it, with the older shader model and the current
//! one, in TextureCraft's terms (colours linear, directions packed). Inputs and expected values only.

use texturecraft_core::Channel;
use texturecraft_import::channels::{SHADER_MODEL_CURRENT, channel_for_bit, uncovered_default, uniform_value};
use texturecraft_import_reader::decode::document::Scalar;
use texturecraft_import_reader::model::ProjectChannel;

/// Where an index lands in the older numbering the reader maps every file to.
enum Old {
    /// The same bit in older files.
    Bit,
    /// No such channel in older files; kept at this bit.
    Moved(u64),
    /// A user channel: this bit in older files.
    User(u64),
    /// No bit at all (user channels 8 and up).
    Dropped,
}

/// (index, older numbering, TextureCraft's channel, colour, uncovered with the older shader model, with the current one).
type Row = (u32, Old, Option<Channel>, bool, [f32; 3], [f32; 3]);

const FLAT: [f32; 3] = [0.498, 0.498, 1.0];

fn grey(v: f32) -> [f32; 3] {
    [v; 3]
}

#[rustfmt::skip]
fn table() -> Vec<Row> {
    use Old::*;
    let mut rows: Vec<Row> = vec![
        (0, Bit, Some(Channel::BaseColor), true, grey(0.498), grey(0.8)),
        (1, Bit, Some(Channel::Height), false, grey(0.0), grey(0.0)),
        (2, Bit, Some(Channel::Specular), true, grey(0.0392), grey(0.0392)),
        (3, Bit, Some(Channel::Opacity), false, grey(1.0), grey(1.0)),
        (4, Bit, Some(Channel::Emissive), true, grey(0.0), grey(1.0)),
        (5, Bit, None, false, grey(0.0), grey(0.0)),
        (6, Bit, Some(Channel::Glossiness), false, grey(0.698), grey(0.698)),
        (7, Bit, Some(Channel::Roughness), false, grey(0.298), grey(0.298)),
        (8, Bit, Some(Channel::AnisotropyLevel), false, grey(0.0), grey(0.0)),
        (9, Bit, Some(Channel::AnisotropyAngle), false, grey(0.0), grey(0.0)),
        (10, Bit, None, true, grey(0.0), grey(0.0)),
        (11, Bit, None, false, grey(0.0), grey(0.0)),
        (12, Bit, None, false, grey(0.498), grey(0.498)),
        (13, Bit, Some(Channel::Metallic), false, grey(0.0), grey(0.0)),
        (14, Moved(1 << 49), None, false, grey(1.0), grey(1.0)),
        (15, Moved(1 << 50), None, false, grey(0.0), grey(0.0)),
        (16, Moved(1 << 51), None, false, grey(1.0), grey(1.0)),
        (17, Moved(1 << 52), Some(Channel::Transmission), false, grey(0.0), grey(0.0)),
        (19, Moved(1 << 54), None, false, grey(0.0), grey(0.0)),
        (20, Moved(1 << 55), None, false, grey(0.0), grey(0.0)),
        (21, Moved(1 << 56), None, false, FLAT, FLAT),
        (22, Bit, Some(Channel::Normal), false, FLAT, FLAT),
        (23, Bit, Some(Channel::AmbientOcclusion), false, grey(1.0), grey(1.0)),
        (24, Bit, None, true, grey(0.0), grey(0.0)),
        (25, Bit, Some(Channel::SpecularLevel), false, grey(0.498), grey(0.498)),
        (26, Bit, None, false, grey(0.0), grey(0.0)),
        (27, Bit, Some(Channel::ScatterWeight), false, grey(0.0), grey(1.0)),
        (28, Bit, Some(Channel::SheenWeight), false, grey(0.0), grey(0.0)),
        (29, Bit, Some(Channel::SheenRoughness), false, grey(0.498), grey(0.498)),
        (30, Bit, Some(Channel::SheenColor), true, grey(1.0), grey(1.0)),
        (33, Bit, None, false, FLAT, FLAT),
        (40, Bit, Some(Channel::CoatWeight), false, grey(0.0), grey(0.0)),
        (41, Bit, None, true, grey(1.0), grey(1.0)),
        (42, Bit, Some(Channel::CoatRoughness), false, grey(0.0), grey(0.0)),
        (43, Bit, None, false, grey(0.498), grey(0.498)),
        (44, Bit, Some(Channel::CoatNormal), false, FLAT, FLAT),
        (45, Bit, None, true, grey(1.0), grey(0.8)),
        (46, Bit, None, false, grey(0.0), grey(0.0)),
        (47, Bit, None, true, grey(1.0), grey(1.0)),
        (48, Bit, None, true, grey(1.0), grey(1.0)),
    ];
    for user in 0..32u32 {
        let old = if user < 8 { User(1 << (14 + user)) } else { Dropped };
        let ours = if user < 8 { Channel::user(user as usize) } else { None };
        rows.push((64 + user, old, ours, false, grey(1.0), grey(1.0)));
    }
    rows
}

/// Where the importer keeps its own value on purpose: (channel, shader model current) → TextureCraft's value.
fn kept_on_purpose(channel: Channel, current: bool) -> bool {
    // Emissive has no separate strength in TextureCraft; scatter weight is not a radius scale.
    current && matches!(channel, Channel::Emissive | Channel::ScatterWeight)
}

#[test]
fn indices_and_bits_name_the_same_channel() {
    for (index, old, ours, colour, ..) in table() {
        let bit = match old {
            Old::Bit => 1u64 << index,
            Old::Moved(b) | Old::User(b) => b,
            Old::Dropped => {
                assert_eq!(Scalar::Enum(u64::from(index)).channel_bit(), None, "index {index}");
                continue;
            }
        };
        assert_eq!(Scalar::Enum(u64::from(index)).channel_bit(), Some(bit), "index {index}");
        assert_eq!(Scalar::WideMask(1u128 << index).channel_mask(), Some(bit), "mask of index {index}");
        if let Old::Bit | Old::User(_) = old {
            assert_eq!(Scalar::ChannelType(bit).channel_bit(), Some(bit), "older bit of index {index}");
        }
        assert_eq!(channel_for_bit(bit), ours, "index {index}");
        let named = ProjectChannel::from_bit(bit);
        assert_eq!(named.is_colour(), colour, "index {index}");
        if let Old::User(_) = old {
            assert!(matches!(named, ProjectChannel::Bit(_)), "index {index} is a user channel");
        } else {
            assert!(!matches!(named, ProjectChannel::Bit(_)), "index {index} is named");
        }
        if let Some(c) = ours {
            assert_eq!(colour, c.kind() == texturecraft_core::ChannelKind::Color, "colour handling of index {index}");
        }
    }
}

#[test]
fn uncovered_channels_show_the_measured_values() {
    for (index, _, ours, _, older, current) in table() {
        let Some(c) = ours else { continue };
        for (model, expected, is_current) in [(None, older, false), (Some(2), older, false), (Some(SHADER_MODEL_CURRENT), current, true)] {
            if kept_on_purpose(c, is_current) {
                continue;
            }
            let ours = uncovered_default(c, model);
            for k in 0..c.components() {
                assert!((ours[k] - expected[k]).abs() <= 1.5 / 255.0, "index {index} {c:?} model {model:?}: {ours:?} vs {expected:?}");
            }
        }
    }
}

#[test]
fn a_grey_channel_shows_the_mean_of_the_stored_components() {
    // (stored, shown) pairs from the fill: a grey channel exported the mean of the three stored values.
    let cases = [([0.372549, 0.627451, 0.435294], 0.478431), ([0.2, 0.8, 0.349020], 0.449673), ([0.823529, 0.176471, 0.662745], 0.554248)];
    for (stored, shown) in cases {
        let v = uniform_value(Channel::Opacity, stored);
        assert!((v[0] - shown).abs() < 1e-5, "{v:?}");
    }
    assert_eq!(uniform_value(Channel::BaseColor, [0.1, 0.2, 0.3]), [0.1, 0.2, 0.3]);
}
