mod app;
mod assets;
pub(crate) mod layout;
mod shell;
mod storage;
mod theme;

fn main() -> anyhow::Result<()> {
    app::run()
}
