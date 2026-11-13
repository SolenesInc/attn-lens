fn main() {
    let mut args = std::env::args().skip(1);
    let result = if args.next().as_deref() == Some("lens") {
        // Talks to a running Lens over the automation socket.
        let line = args.next().unwrap_or_else(|| r#"{"cmd":"state"}"#.into());
        attn_lens::lens::automation::socket_path()
            .and_then(|path| attn_lens::lens::automation::request(&path, &line))
            .map(|reply| println!("{reply}"))
    } else {
        attn_lens::app::run()
    };
    if let Err(error) = result {
        eprintln!("Attn Lens: {error}");
        std::process::exit(1);
    }
}
