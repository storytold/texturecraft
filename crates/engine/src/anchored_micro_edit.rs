//! `microAnchor` and `microAmount`: the earlier commands for a generator's micro detail, now its Micro Height read from an anchor point

use serde_json::Value;
use texturecraft_core::{Channel, Generator, LayerId, MicroDetails, Source};

use crate::commands::bad;
use crate::mask_cmds::anchor_source;
use crate::{Result, Session};

/// What `microAnchor` and `microAmount` change on a generator's micro details
pub struct AnchoredMicroEdit {
    /// The anchor point the Micro Height reads (`Some(None)`: `microAnchor: null` empties the slot)
    anchor: Option<Option<Source>>,
    /// `microAmount`, 0..1, read as the Height Details Intensity
    amount: Option<f32>,
}

impl AnchoredMicroEdit {
    /// The edit `params` ask of a generator on layer `reader` (None when they give neither key)
    pub fn parse(session: &Session, command: &str, reader: LayerId, params: &Value) -> Result<Option<AnchoredMicroEdit>> {
        let amount = match params.get("microAmount") {
            Some(value) => {
                let amount = value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| bad(command, "`microAmount` must be a number 0..1"))?;
                Some((amount as f32).clamp(0.0, 1.0))
            }
            None => None,
        };
        let anchor = match params.get("microAnchor") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(value) => {
                let anchor = value.as_u64().ok_or_else(|| bad(command, "`microAnchor` must be an anchor effect id or null"))?;

                // The same rule as fills: an anchor below, in the same texture set
                Some(Some(anchor_source(session, command, reader, LayerId(anchor), Channel::Height)?))
            }
        };
        if anchor.is_none() && amount.is_none() {
            return Ok(None);
        }

        Ok(Some(AnchoredMicroEdit { anchor, amount }))
    }

    /// The edit made on `generator`: an anchor switches the micro details and their Micro Height on and reads it there; a generator on the earlier model reads it as these keys did before Micro Details
    pub fn apply(self, command: &str, generator: &mut Generator) -> Result<()> {
        let kind = generator.kind;
        let earlier = generator.on_earlier_model();
        let defaults = MicroDetails::defaults(kind);
        match (self.anchor, &defaults) {
            (Some(None), _) => {
                if let Some(micro) = generator.micro.as_mut() {
                    micro.height_source = None;
                }
            }
            (Some(Some(source)), Some(defaults)) => {
                let micro = generator.micro.get_or_insert_with(|| Box::new(defaults.clone()));
                (micro.enabled, micro.height, micro.height_source) = (true, true, Some(source));
            }
            (Some(Some(_)), None) => return Err(bad(command, format!("{} has no micro details for `microAnchor`", kind.label()))),
            (None, _) => {}
        }

        if let Some(amount) = self.amount {
            let reads_height = generator.micro.as_mut().filter(|micro| micro.height_source.is_some());
            let micro = reads_height.ok_or_else(|| bad(command, "give `microAnchor` before `microAmount`: the generator's Micro Height reads nothing"))?;
            micro.height_intensity = amount * MicroDetails::HEIGHT_INTENSITY_PER_AMOUNT;
            **micro = micro.as_ref().clone().clamped(kind);
        }
        if let Some(micro) = generator.micro.as_mut() {
            micro.earlier = earlier;
        }
        Ok(())
    }
}
