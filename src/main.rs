extern crate gtk;
extern crate rand;

use gtk::prelude::*;
use rand::Rng;
use std::ffi::OsString;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;

const PROGRAM_NAME: &str = "options-window-gtk";
const VERSION: &str = "0.1.0";
const DEFAULT_TERMINAL: &str = "i3-sensible-terminal";

#[derive(PartialEq, Clone)]
enum ParseErrorType {
    HelpRequested,
    VersionInfoRequested,
    MissingArgument,
    WrongArgument,
}

pub struct ParseError {
    message: String,
    error_type: ParseErrorType,
}

impl ParseError {
    pub fn missing_argument<T: Into<String>>(msg: T) -> Self {
        ParseError {
            error_type: ParseErrorType::MissingArgument,
            message: msg.into(),
        }
    }

    pub fn wrong_argument<T: Into<String>>(msg: T) -> Self {
        ParseError {
            error_type: ParseErrorType::WrongArgument,
            message: msg.into(),
        }
    }

    pub fn help_requested() -> Self {
        ParseError {
            error_type: ParseErrorType::HelpRequested,
            message: "".into(),
        }
    }

    pub fn version_requested() -> Self {
        ParseError {
            error_type: ParseErrorType::VersionInfoRequested,
            message: "".into(),
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::fmt::Debug for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[derive(Clone, Debug)]
enum MessageType {
    WARNING,
    ERROR,
}

type CommandFunction = fn(&Command) -> std::io::Result<std::process::Child>;

#[derive(Clone)]
pub struct Command {
    command: OsString,
    exec: CommandFunction,
}

impl Command {
    pub fn new(command: OsString, exec: CommandFunction) -> Self {
        Self { command, exec }
    }

    pub fn execute(&self) {
        (self.exec)(&self).expect("Failed to spawn child process.");
    }
}

fn exec_in_shell(command: &Command) -> std::io::Result<std::process::Child> {
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&command.command)
        .spawn()
}

/* The method used here is roughly the same as in i3-nagbar:
 * A temporary script with the command and a link to this executable is created.
 * Afterwards the terminal emulator gets called with -e <link>
 * If this executable gets called with a '.cmd' ending it starts a shell with the
 * script as parameter.
 *
 * The reason for this is that not all terminal emulators handle -e the same way.
 *
 * There might be some security issues with this...
*/
fn exec_in_terminal(command: &Command) -> std::io::Result<std::process::Child> {
    let tmpdir = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(v) => std::path::PathBuf::from(v),
        None => std::env::temp_dir(),
    };
    let mut script_path = tmpdir.clone();
    let mut link_path = tmpdir.clone();

    let rnd: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(30)
        .collect();
    let script_name = OsString::from(format!("{}_{}.sh", PROGRAM_NAME, rnd));
    script_path.push(script_name);
    let link_name = OsString::from(format!("{}_{}.cmd", PROGRAM_NAME, rnd));
    link_path.push(link_name);
    {
        let mut script_file = std::fs::File::create(&script_path)?;
        let mut p = script_file.metadata()?.permissions();
        p.set_mode(0o700);
        script_file
            .set_permissions(p)
            .expect("Couldn't set permissions for script file");
        script_file.write_all(b"#!/bin/sh\n")?;
        script_file.write_all(b"rm ")?;
        script_file.write_all(&script_path.as_os_str().as_bytes())?;
        script_file.write_all(b"\n")?;
        script_file.write_all(command.command.as_bytes())?;
        script_file.write_all(b"\n")?;
        script_file.flush()?;
    }
    std::os::unix::fs::symlink(std::env::current_exe()?, &link_path)?;
    std::process::Command::new(DEFAULT_TERMINAL)
        .arg("-v")
        .arg("-e")
        .arg(link_path.as_os_str())
        .spawn()
}

#[derive(Clone)]
pub struct Button {
    label: String,
    icon: Option<OsString>,
    command: Command,
}

#[derive(Clone)]
pub struct Configuration {
    message: String,
    exit_after_action: bool,
    message_type: MessageType,
    buttons: Vec<Button>,
}

impl Configuration {
    pub fn new(args: &[OsString]) -> Result<Self, ParseError> {
        let mut config = Configuration {
            buttons: Vec::new(),
            message_type: MessageType::ERROR,
            exit_after_action: false,
            message: String::from("This could be your text!"),
        };

        let mut pos = 1;
        while pos < args.len() {
            let a = &args[pos];
            if a.eq("-m") || a.eq("--message") {
                pos += 1;
                let msg_opt = Configuration::get_argument(pos, &args);
                if msg_opt.is_none() {
                    return Err(ParseError::missing_argument(
                        "Required argument for -m is missing.",
                    ));
                }
                config.message = String::from(msg_opt.unwrap().to_string_lossy());
            } else if a.eq("-t") || a.eq("--type") {
                pos += 1;
                let type_opt = Configuration::get_argument(pos, &args);
                if type_opt.is_none() {
                    return Err(ParseError::missing_argument(
                        "Required argument for -t is missing.",
                    ));
                }
                let msg_type = type_opt.unwrap().to_string_lossy();
                if msg_type.eq_ignore_ascii_case("warning") {
                    config.message_type = MessageType::WARNING;
                } else if !msg_type.eq_ignore_ascii_case("error") {
                    return Err(ParseError::wrong_argument(format!(
                        "Parameter for -t ({}) was neither warning nor error.",
                        msg_type
                    )));
                }
            } else if a.eq("-b") || a.eq("--button") {
                let button = Configuration::create_button(&mut pos, args, exec_in_terminal)?;
                config.buttons.push(button);
            } else if a.eq("-B") || a.eq("--button-no-terminal") {
                let button = Configuration::create_button(&mut pos, &args, exec_in_shell)?;
                config.buttons.push(button);
            } else if a.eq("--exit-after-action") {
                config.exit_after_action = true;
            } else if a.eq("-f") || a.eq("--font") {
                pos += 1
            // don't handle fonts...
            } else if a.eq("-h") || a.eq("--help") {
                return Err(ParseError::help_requested());
            } else if a.eq("-v") || a.eq("--version") {
                return Err(ParseError::version_requested());
            } else {
                return Err(ParseError::wrong_argument(format!(
                    "Unexpected argument: {}",
                    a.to_string_lossy()
                )));
            }
            pos += 1;
        }
        Ok(config)
    }

    fn create_button(
        pos: &mut usize,
        args: &[OsString],
        cmd_func: CommandFunction,
    ) -> Result<Button, ParseError> {
        *pos += 1;
        let label_opt = Configuration::get_argument(*pos, &args);
        if label_opt.is_none() {
            return Err(ParseError::missing_argument("Missing label for Button."));
        }
        let label = label_opt.unwrap().to_string_lossy().to_string();
        *pos += 1;

        let action_opt = Configuration::get_argument(*pos, &args);
        if action_opt.is_none() {
            return Err(ParseError::missing_argument("Missing action for Button."));
        }
        let action = action_opt.unwrap().clone();
        let icon = match Configuration::get_argument(*pos + 1, &args) {
            Some(v) => {
                if v.as_bytes().starts_with(b"-") {
                    None
                } else {
                    *pos += 1;
                    Some(v.clone())
                }
            }
            None => None,
        };
        let button = Button {
            label,
            icon,
            command: Command::new(action, cmd_func),
        };
        Ok(button)
    }

    fn get_argument<P>(pos: usize, args: &[P]) -> Option<&P> {
        if pos < args.len() {
            return Some(&args[pos]);
        }
        None
    }
}

fn create_gtk_window(
    buttons: &gtk::Box,
    default_button: &gtk::Button,
    message: &gtk::Box,
) -> gtk::Window {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_keep_above(true);
    window.stick();
    window.set_urgency_hint(true);
    window.set_title(PROGRAM_NAME);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 5);
    content.add(message);
    content.add(buttons);
    window.set_border_width(10);
    window.set_position(gtk::WindowPosition::Center);
    window.add(&content);
    window.set_resizable(false);
    default_button.set_can_default(true);
    window.set_default(default_button);
    window.activate_focus();
    default_button.grab_focus();
    window
}

fn create_gtk_message(config: &Configuration) -> gtk::Box {
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let icon = match config.message_type {
        MessageType::ERROR => gtk::Image::new_from_icon_name("dialog-error", 6),
        MessageType::WARNING => gtk::Image::new_from_icon_name("dialog-warning", 6),
    };
    let label = gtk::Label::new(config.message.as_str());
    hbox.add(&icon);
    hbox.add(&label);
    hbox
}

fn create_gtk_button(caption: &str, icon: &Option<OsString>) -> gtk::Button {
    let gtk_button = gtk::Button::new();
    let b_box = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let label = gtk::Label::new_with_mnemonic(caption);
    label.set_halign(gtk::Align::Center);
    b_box.pack_start(&label, true, true, 0);
    if let Some(v) = &icon {
        let image = gtk::Image::new_from_icon_name(v.to_string_lossy().as_ref(), 4);
        image.set_halign(gtk::Align::Start);
        b_box.pack_end(&image, false, true, 0);
    }
    gtk_button.add(&b_box);
    gtk_button
}

fn create_gtk_buttons(config: &Configuration) -> (gtk::Box, gtk::Button) {
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 5);

    for button in &config.buttons {
        let gtk_button = create_gtk_button(button.label.as_str(), &button.icon);
        vbox.pack_start(&gtk_button, true, true, 0);
        let button_clone = button.clone();
        if config.exit_after_action {
            gtk_button.connect_clicked(move |_| {
                button_clone.command.execute();
                gtk::main_quit();
            });
        } else {
            gtk_button.connect_clicked(move |_| {
                button_clone.command.execute();
            });
        }
    }
    let button2 = create_gtk_button("_Cancel", &Some(OsString::from("window-close")));
    button2.connect_clicked(|_| {
        gtk::main_quit();
    });
    vbox.add(&button2);
    (vbox, button2)
}

fn show_version() {
    println!("{} {}", PROGRAM_NAME, VERSION);
}

fn usage_short() {
    println!("Usage: {} [-h] [-v] [-b label action [icon]]... [-B label action [icon]]... [-t warning|error] [-m message] [-f font]", PROGRAM_NAME);
}

fn usage_long() {
    println!("Usage:");
    println!("  {} [OPTION]...", PROGRAM_NAME);
    println!();
    println!("Options:");
    println!("  -h, --help                                     Prints help information");
    println!("  -v, --version                                  Prints version information");
    println!("  -b, --button LABEL ACTION [ICON]               Creates a button.");
    println!("  -B, --button-no-terminal LABEL ACTION [ICON]   Creates a button.");
    println!("  -m, --message MSG                              Sets the window caption");
    println!(
        "  -t, --type warning|error                       Default: error. Defines the window icon"
    );
    println!("  --exit-after-action                            Program exits after a button press");
}

fn show_error(error: ParseError) {
    println!("Error while parsing command line: {}", error);
}

fn show_help() {
    show_version();
    usage_long();
}

fn run_script(cmd: &OsString) {
    let mut script = OsString::from_vec(cmd.as_bytes()[..cmd.len() - 3].to_vec());
    script.push("sh");
    std::process::Command::new("/bin/sh")
        .arg(script)
        .spawn()
        .expect("Couldn't spawn child process.")
        .wait()
        .expect("Error during childs execute.");
}

fn handle_error(err: ParseError) -> i32 {
    let mut exit_code = 0;
    if err.error_type == ParseErrorType::HelpRequested {
        show_help();
    } else if err.error_type == ParseErrorType::VersionInfoRequested {
        show_version();
    } else {
        show_error(err);
        usage_short();
        exit_code = 1;
    }
    exit_code
}

fn main() {
    let mut exit_code: i32 = 0;
    let args = std::env::args_os().collect::<Vec<OsString>>();
    if !args[0].to_string_lossy().ends_with(".cmd") {
        let result = Configuration::new(&args);
        if result.is_ok() {
            let config = result.unwrap();
            gtk::init().expect("Couldn't start gtk.");
            let (gtk_buttons, default) = create_gtk_buttons(&config);
            let gtk_message = create_gtk_message(&config);
            let window = create_gtk_window(&gtk_buttons, &default, &gtk_message);
            window.show_all();
            gtk::main();
        } else {
            let err = result.err().unwrap();
            exit_code = handle_error(err);
        }
    } else {
        match std::fs::remove_file(&args[0]) {
            Ok(_) => {}
            Err(e) => println!("Couldn't delete link {}\n{}", &args[0].to_string_lossy(), e),
        }
        run_script(&args[0]);
    }
    std::process::exit(exit_code);
}

#[cfg(test)]
mod tests {
    use crate::Configuration;
    use std::ffi::OsString;

    fn o(s: &str) -> OsString {
        OsString::from(s)
    }
    #[test]
    fn button_order() {
        let args = vec![
            o("app"),
            o("-b"),
            o("1.1"),
            o("1.2"),
            o("-B"),
            o("2.1"),
            o("2.2"),
            o("-b"),
            o("3.1"),
            o("3.2"),
            o("-B"),
            o("4.1"),
            o("4.2"),
        ];
        let config = Configuration::new(&args).unwrap();
        let mut i = 1;
        for button in config.buttons {
            let label = format!("{}.1", i);
            assert_eq!(label, button.label);
            i += 1;
        }
    }
}
