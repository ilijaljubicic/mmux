use std::ffi::OsString;

fn main() {
    let mut args: Vec<OsString> = std::env::args_os().collect();

    match args.get(1).and_then(|arg| arg.to_str()) {
        Some("controller") => {
            args.remove(1);
            mmux_controller::main_entry_from(args);
        }
        Some("node") => {
            args.remove(1);
            mmux_node::main_entry_from(args);
        }
        _ => {
            mmux_controller::main_entry_from(args);
        }
    }
}
