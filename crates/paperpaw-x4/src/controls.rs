//! What the X4 shells change by the power level (ADR 0025), as plugins see
//! it in `Capabilities::power`, so that the power settings show only what
//! applies on the device.
//!
//! - `buttons`: how often the buttons are read follows the level (every
//!   50 ms in Saving and Offline), on all of them.
//! - `frontlight`: the X4 Pro's light follows it, dimmer and off sooner
//!   (the modes' presets set `x4.frontlight`); only the Pro has a light.
//! - Not `radio`: the radio's own saving follows whether the network is
//!   busy, not the level. Not `cpu_clock`: the clock is fixed at the maximum.

use paperpaw_sdk::power::PowerControls;

/// The controls of a device, `frontlight` where it has a front light.
pub fn power_controls(frontlight: bool) -> PowerControls {
    let mut controls = PowerControls::default();
    controls.buttons = true;
    controls.frontlight = frontlight;
    controls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_device_slows_its_buttons_and_only_the_pro_dims_a_light() {
        let plain = power_controls(false);
        assert!(plain.any() && plain.buttons && !plain.frontlight);
        let pro = power_controls(true);
        assert!(pro.buttons && pro.frontlight);
        for controls in [plain, pro] {
            assert!(!controls.radio && !controls.cpu_clock && !controls.system_saver);
        }
    }
}
