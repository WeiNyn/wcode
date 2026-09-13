//! Feel-the-skeleton binary: `cargo run -p wcode-tui --example demo`.
//! Type a line, Enter to commit it, Esc (or Ctrl-C) to quit.

#[tokio::main]
async fn main() -> std::io::Result<()> {
    wcode_tui::run().await
}
