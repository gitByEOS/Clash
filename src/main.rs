mod api_test;
mod cli;
mod config;
mod crypto;
mod fuzzy;
mod ops;
mod statusline;
mod tui;

fn main() {
    cli::launch();
}