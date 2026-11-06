mod app;
mod assets;
mod shell;
mod theme;

fn main() -> anyhow::Result<()> {
    app::run()
}
