fn main() {
    if let Err(error) = yu_shell_windows::run() {
        eprintln!("Yu: {error}");
        std::process::exit(1);
    }
}
