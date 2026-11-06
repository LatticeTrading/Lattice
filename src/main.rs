mod app;
mod assets;
mod layout;
mod shell;
mod theme;

fn main() -> anyhow::Result<()> {
    app::run()
}
