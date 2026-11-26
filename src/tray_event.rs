//! Decoding of the tray icon callback message (pure logic, testable anywhere).

pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_CONTEXTMENU: u32 = 0x007B;
pub const NIN_SELECT: u32 = 0x0400;
pub const NIN_KEYSELECT: u32 = 0x0401;

/// What the app should do for a tray callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenOverview,
    ShowMenu,
    Ignore,
}

/// Map the `lParam` of the tray callback message to an action.
///
/// `version4` is true when the icon runs with `NOTIFYICON_VERSION_4`: then the
/// event is in the low word of `lParam` and the menu arrives as
/// `WM_CONTEXTMENU` (right click and keyboard), the primary click as
/// `NIN_SELECT`/`NIN_KEYSELECT`. Raw button messages are ignored there so a
/// click never triggers two actions. In legacy mode `lParam` is the mouse
/// message itself.
pub fn decode(lparam: isize, version4: bool) -> TrayAction {
    let event = (lparam as usize as u32) & 0xFFFF;
    if version4 {
        match event {
            WM_CONTEXTMENU => TrayAction::ShowMenu,
            NIN_SELECT | NIN_KEYSELECT => TrayAction::OpenOverview,
            _ => TrayAction::Ignore,
        }
    } else {
        match event {
            WM_RBUTTONUP | WM_CONTEXTMENU => TrayAction::ShowMenu,
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => TrayAction::OpenOverview,
            _ => TrayAction::Ignore,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_left_opens_overview() {
        assert_eq!(
            decode(WM_LBUTTONUP as isize, false),
            TrayAction::OpenOverview
        );
        assert_eq!(
            decode(WM_LBUTTONDBLCLK as isize, false),
            TrayAction::OpenOverview
        );
    }

    #[test]
    fn legacy_right_and_context_menu_show_menu() {
        assert_eq!(decode(WM_RBUTTONUP as isize, false), TrayAction::ShowMenu);
        assert_eq!(decode(WM_CONTEXTMENU as isize, false), TrayAction::ShowMenu);
    }

    #[test]
    fn v4_packing_uses_low_word_only() {
        // HIWORD carries the icon id.
        let ctx = ((1isize) << 16) | WM_CONTEXTMENU as isize;
        let sel = ((1isize) << 16) | NIN_SELECT as isize;
        assert_eq!(decode(ctx, true), TrayAction::ShowMenu);
        assert_eq!(decode(sel, true), TrayAction::OpenOverview);
        assert_eq!(
            decode(NIN_KEYSELECT as isize, true),
            TrayAction::OpenOverview
        );
    }

    #[test]
    fn v4_ignores_raw_buttons_to_avoid_double_actions() {
        assert_eq!(decode(WM_LBUTTONUP as isize, true), TrayAction::Ignore);
        assert_eq!(decode(WM_RBUTTONUP as isize, true), TrayAction::Ignore);
    }

    #[test]
    fn unrelated_messages_are_ignored() {
        for v4 in [false, true] {
            assert_eq!(decode(0x0200, v4), TrayAction::Ignore); // WM_MOUSEMOVE
            assert_eq!(decode(0x0201, v4), TrayAction::Ignore); // WM_LBUTTONDOWN
            assert_eq!(decode(0x0204, v4), TrayAction::Ignore); // WM_RBUTTONDOWN
            assert_eq!(decode(0, v4), TrayAction::Ignore);
        }
    }
}
