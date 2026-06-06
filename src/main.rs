mod api_test;
mod claude;
mod cli;
mod config;
mod crypto;
mod fuzzy;
mod ops;
mod prompts;
mod statusline;
mod tui;

fn main() {
    cli::launch();
}