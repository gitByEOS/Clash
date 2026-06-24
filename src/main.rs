mod api_test;
mod chat;
mod claude;
mod claude_history;
mod cli;
mod config;
mod crypto;
mod fuzzy;
mod hooks;
mod lark;
mod model;
mod ops;
mod prompt_capture;
mod prompts;
mod statusline;
mod tui;

fn main() {
    cli::launch();
}
