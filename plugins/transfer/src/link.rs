//! The address a server of this plugin is opened at: the text shown, and
//! what the QR code carries.
//!
//! Where the shell gives the device a name in the network
//! (`WifiState::reach`, e.g. `paperpaw.local`), the text shows it, as it
//! is easier to type. The QR code keeps the numeric address: a phone's
//! name lookup (mDNS) is not sure everywhere, and scanning needs no typing.

use alloc::format;
use alloc::string::String;

use paperpaw_sdk::wifi::WifiState;

/// The address of a server that started at `address` (`192.168.1.20:8080`,
/// the port absent for the usual one), with `key`: `(shown, scanned)`.
/// `wifi` is the radio's state where PaperPaw manages it.
pub(crate) fn urls(wifi: Option<&WifiState>, address: &str, key: &str) -> (String, String) {
    let scanned = format!("http://{address}/?k={key}");
    let Some(reach) = wifi.and_then(WifiState::reach) else {
        return (scanned.clone(), scanned);
    };
    let port = match address.rsplit_once(':') {
        Some((_, port)) => format!(":{port}"),
        None => String::new(),
    };
    (format!("http://{reach}{port}/?k={key}"), scanned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn online(name: Option<&str>) -> WifiState {
        WifiState::Online {
            ssid: "home".into(),
            address: "192.168.1.20".into(),
            name: name.map(String::from),
        }
    }

    #[test]
    fn the_name_is_shown_and_the_address_is_scanned() {
        let state = online(Some("paperpaw.local"));
        let (shown, scanned) = urls(Some(&state), "192.168.1.20:8080", "abc");
        assert_eq!(shown, "http://paperpaw.local:8080/?k=abc");
        assert_eq!(scanned, "http://192.168.1.20:8080/?k=abc");
    }

    #[test]
    fn a_usual_port_is_left_out() {
        let state = online(Some("paperpaw-x4-1a2b.local"));
        let (shown, scanned) = urls(Some(&state), "192.168.1.20", "k");
        assert_eq!(shown, "http://paperpaw-x4-1a2b.local/?k=k");
        assert_eq!(scanned, "http://192.168.1.20/?k=k");
    }

    #[test]
    fn without_a_name_the_address_is_both() {
        for state in [Some(online(None)), Some(online(Some(""))), None] {
            let (shown, scanned) = urls(state.as_ref(), "192.168.1.20:8080", "k");
            assert_eq!(shown, "http://192.168.1.20:8080/?k=k");
            assert_eq!(scanned, shown);
        }
    }

    #[test]
    fn the_own_network_shows_its_address() {
        let state = WifiState::Hosting {
            ssid: "PaperPaw".into(),
            address: "192.168.4.1".into(),
            guests: 1,
        };
        let (shown, scanned) = urls(Some(&state), "192.168.4.1:8080", "k");
        assert_eq!(shown, "http://192.168.4.1:8080/?k=k");
        assert_eq!(scanned, shown);
    }
}
