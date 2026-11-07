fn main() {
    if std::env::args().any(|arg| arg == "fail") {
        eprintln!("mock SSH: forwarding failed");
        std::process::exit(7);
    }
    if std::env::args().any(|arg| arg == "exit-later") {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        return;
    }
    std::thread::sleep(std::time::Duration::from_secs(120));
}
