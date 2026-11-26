//! The pointer keeper's command line on Windows: whose pick it serves (the app's process id), the event that ends the pick and
//! the dots per inch of the app's window, which size the eyedropper pointer as the system sizes its own pointers there

#[cfg(any(windows, test))]
use std::ffi::OsString;

#[cfg(any(windows, test))]
use super::cursor_image::SIZES;

/// The argument that starts this program as the pointer keeper
pub const KEEPER_ARG: &str = "--eyedropper-pointer";

/// The screen density of a system pointer's nominal size (32 pixels at 100 %)
pub(crate) const BASE_DPI: u32 = 96;
/// The nominal pointer cell at `BASE_DPI`
#[cfg(any(windows, test))]
const BASE_POINTER_PX: u32 = 32;
/// The densest screen taken from a command line (Windows scales to 500 %)
#[cfg(any(windows, test))]
const MAX_DPI: u32 = BASE_DPI * 5;

/// The shown event's name after the stop event's
#[cfg(any(windows, test))]
const SHOWN_SUFFIX: &str = "-shown";
/// The longest stop event name taken (the app's are about 50 characters): with its suffix it stays under the system's 260
#[cfg(any(windows, test))]
const MAX_EVENT_NAME_LEN: usize = 200;

/// What a pointer keeper is started with
#[cfg(any(windows, test))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeeperArgs {
    /// The app's process: the keeper puts the user's pointers back when it ends, however it ends
    pub parent: u32,
    /// The name of the event the app sets when the pick ends
    pub stop: String,
    /// The app window's dots per inch
    pub dpi: u32,
}

#[cfg(any(windows, test))]
impl KeeperArgs {
    /// The whole command line that starts `program` as this keeper (Windows paths hold no quotes)
    pub fn command_line(&self, program: &str) -> String {
        format!("\"{program}\" {KEEPER_ARG} {} {} {}", self.parent, self.stop, self.dpi)
    }

    /// The keeper's arguments after `KEEPER_ARG`, or `None` for anything else
    pub fn parse(args: &[String]) -> Option<Self> {
        let [parent, stop, dpi] = args else { return None };
        let stop = stop.clone();
        let usable_name = !stop.is_empty()
            && stop.len() <= MAX_EVENT_NAME_LEN
            && stop.chars().all(|character| character.is_ascii_alphanumeric() || "\\-_".contains(character));
        let dpi = dpi.parse::<u32>().ok().filter(|dpi| *dpi > 0)?.min(MAX_DPI);

        Some(KeeperArgs { parent: parent.parse().ok()?, stop: usable_name.then_some(stop)?, dpi })
    }

    /// The keeper's arguments after `KEEPER_ARG` as the system gives them; `None` also for one that is not Unicode (no panic)
    pub fn parse_os(args: impl IntoIterator<Item = OsString>) -> Option<Self> {
        let args = args.into_iter().map(|arg| arg.into_string().ok()).collect::<Option<Vec<String>>>()?;
        Self::parse(&args)
    }

    /// The name of the event the keeper sets once the eyedropper shows: from then on the app may change the pointer's picture
    pub fn shown_event(&self) -> String {
        format!("{}{SHOWN_SUFFIX}", self.stop)
    }

    /// The pointer cell in pixels: the system's own pointer size at this density (`system`, where it says), else 32 scaled
    pub fn pointer_px(&self, system: Option<i32>) -> u32 {
        let scaled = BASE_POINTER_PX * self.dpi / BASE_DPI;
        system.and_then(|px| u32::try_from(px).ok()).filter(|px| SIZES.contains(px)).unwrap_or(scaled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &[&str]) -> Vec<String> {
        line.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn the_keeper_reads_back_the_command_line_the_app_writes() {
        let keeper = KeeperArgs { parent: 4120, stop: "Local\\texturecraft-eyedropper-4120-3".into(), dpi: 144 };
        let line = keeper.command_line("C:\\Program Files\\TextureCraft\\texturecraft.exe");
        let after_arg = line.split_once(KEEPER_ARG).map(|(_, rest)| rest.split_whitespace().map(str::to_owned).collect::<Vec<_>>());

        assert!(line.starts_with("\"C:\\Program Files\\TextureCraft\\texturecraft.exe\" "), "the program's path is quoted: {line}");
        assert_eq!(after_arg.and_then(|rest| KeeperArgs::parse(&rest)), Some(keeper.clone()));
        assert_eq!(keeper.shown_event(), "Local\\texturecraft-eyedropper-4120-3-shown", "the shown event is the pick's own");
    }

    #[test]
    fn a_hostile_command_line_starts_no_keeper() {
        assert_eq!(KeeperArgs::parse(&args(&[])), None);
        assert_eq!(KeeperArgs::parse(&args(&["12", "stop"])), None, "too few");
        assert_eq!(KeeperArgs::parse(&args(&["12", "stop", "96", "x"])), None, "too many");
        assert_eq!(KeeperArgs::parse(&args(&["-1", "stop", "96"])), None, "no such process");
        assert_eq!(KeeperArgs::parse(&args(&["12", "stop", "0"])), None, "no density");
        assert_eq!(KeeperArgs::parse(&args(&["12", "a b", "96"])), None, "an event name with a space");
        assert_eq!(KeeperArgs::parse(&args(&["12", "stop", "99999"])).map(|keeper| keeper.dpi), Some(MAX_DPI), "capped");
    }

    #[test]
    fn a_command_line_argument_that_is_not_unicode_starts_no_keeper() {
        #[cfg(unix)]
        let not_unicode = std::os::unix::ffi::OsStringExt::from_vec(vec![0xFF]);
        #[cfg(windows)]
        let not_unicode = std::os::windows::ffi::OsStringExt::from_wide(&[0xD800]);
        let line = |stop: OsString| [OsString::from("12"), stop, OsString::from("96")];

        assert_eq!(KeeperArgs::parse_os(line(not_unicode)), None, "refused, not a panic");
        assert_eq!(
            KeeperArgs::parse_os(line(OsString::from("stop"))),
            Some(KeeperArgs { parent: 12, stop: "stop".into(), dpi: 96 }),
            "plain arguments read as they are"
        );
    }

    #[test]
    fn the_pointer_is_the_systems_pointer_size_at_the_windows_density() {
        let at = |dpi| KeeperArgs { parent: 1, stop: "s".into(), dpi };

        assert_eq!(at(96).pointer_px(Some(32)), 32, "the system's size wins");
        assert_eq!(at(144).pointer_px(Some(48)), 48);
        assert_eq!(at(144).pointer_px(None), 48, "else 32 pixels at 100 %, scaled");
        assert_eq!(at(120).pointer_px(Some(0)), 40, "a size the system can't mean is not taken");
        assert_eq!(at(96).pointer_px(Some(100_000)), 32);
    }
}
