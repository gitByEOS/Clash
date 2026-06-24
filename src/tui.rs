use crate::claude_history::{ClaudeSession, SessionScope};
use crate::fuzzy;
use crossterm::{
    cursor::{MoveToColumn, MoveUp, RestorePosition, SavePosition, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Attribute, Color, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor},
    terminal::{self, size, Clear, ClearType},
    tty::IsTty,
};
use std::io::{self, stdout, Write};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// 按运行 shell 选择配色，bash 下用 ANSI 避免 RGB 在部分终端发偏色
struct Theme {
    prompt: Color,
    text: Color,
    dim: Color,
    hint: Color,
    select: Color,
    select_bg: Color,
    match_color: Color,
}

impl Theme {
    fn detect() -> Self {
        if is_bash_shell() {
            Self::for_bash()
        } else {
            Self::default_rgb()
        }
    }

    fn default_rgb() -> Self {
        Self {
            prompt: Color::Rgb {
                r: 86,
                g: 156,
                b: 214,
            },
            text: Color::Rgb {
                r: 212,
                g: 212,
                b: 212,
            },
            dim: Color::Rgb {
                r: 110,
                g: 110,
                b: 110,
            },
            hint: Color::Rgb {
                r: 78,
                g: 201,
                b: 176,
            },
            select: Color::Rgb {
                r: 255,
                g: 0,
                b: 128,
            },
            select_bg: Color::Rgb {
                r: 55,
                g: 20,
                b: 45,
            },
            match_color: Color::Rgb {
                r: 255,
                g: 215,
                b: 0,
            },
        }
    }

    fn for_bash() -> Self {
        Self {
            prompt: Color::Cyan,
            text: Color::White,
            dim: Color::DarkGrey,
            hint: hint_color(),
            select: Color::Magenta,
            select_bg: Color::AnsiValue(53),
            match_color: Color::Yellow,
        }
    }
}

/// 说明行颜色：truecolor 用自定义浅绿，否则走终端 ANSI 色表里的绿
fn hint_color() -> Color {
    if supports_truecolor() {
        Color::Rgb {
            r: 78,
            g: 201,
            b: 176,
        }
    } else {
        Color::AnsiValue(10)
    }
}

fn supports_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| {
            let lower = v.to_lowercase();
            lower.contains("truecolor") || lower.contains("24bit")
        })
        .unwrap_or(false)
}

fn is_bash_shell() -> bool {
    std::env::var("SHELL")
        .map(|shell| {
            shell
                .rsplit('/')
                .next()
                .unwrap_or(&shell)
                .starts_with("bash")
        })
        .unwrap_or(false)
}
/// 选中标记区固定列数，与终端实际渲染宽度解耦
const MARKER_END_COL: u16 = 2;

// fzf-style model selector.
// Layout (alternate screen):
// clash> sonnet
// 1/3 ─────────────────────────
// 选择模型 | ↑/↓ 选择, Enter 确认, Esc 退出
// → model  claude-sonnet-4-20250514

pub fn select_model(models: &[String]) -> Option<String> {
    select_item(models, "选择模型")
}

pub fn select_item(items: &[String], title: &str) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    if !io::stdin().is_tty() {
        return Some(items[0].clone());
    }

    let result = run_tui(items, title);

    // Always restore terminal on any exit path
    let _ = execute!(
        stdout(),
        ResetColor,
        SetAttribute(Attribute::Reset),
        Show,
        Clear(ClearType::FromCursorDown)
    );
    let _ = terminal::disable_raw_mode();
    result
}

pub fn select_resume_session(current: &[ClaudeSession], all: &[ClaudeSession]) -> Option<String> {
    if current.is_empty() && all.is_empty() {
        return None;
    }
    if !io::stdin().is_tty() {
        return current
            .first()
            .or_else(|| all.first())
            .map(|session| session.id.clone());
    }

    let result = run_resume_tui(current, all);

    let _ = execute!(
        stdout(),
        ResetColor,
        SetAttribute(Attribute::Reset),
        Show,
        Clear(ClearType::FromCursorDown)
    );
    let _ = terminal::disable_raw_mode();
    result
}

struct State {
    all_items: Vec<String>,
    query: String,
    cursor_pos: usize,
    selected: usize,
    offset: usize,
    filtered: Vec<String>,
    height: usize,
    width: usize,
    /// 上一帧渲染的总行数
    total_lines: usize,
    theme: Theme,
    title: String,
}

impl State {
    fn new(items: &[String], title: &str) -> Self {
        let mut s = Self {
            all_items: items.to_vec(),
            query: String::new(),
            cursor_pos: 0,
            selected: 0,
            offset: 0,
            filtered: items.to_vec(),
            height: 0,
            width: 80,
            total_lines: 0,
            theme: Theme::detect(),
            title: title.to_string(),
        };
        s.update_filtered();
        s
    }

    fn update_filtered(&mut self) {
        if self.query.is_empty() {
            self.filtered = self.all_items.clone();
        } else {
            let mut scored: Vec<(i64, String)> = self
                .all_items
                .iter()
                .filter_map(|m| fuzzy::fuzzy_score(&self.query, m).map(|s| (s, m.clone())))
                .collect();
            scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
            self.filtered = scored.into_iter().map(|(_, m)| m).collect();
        }
        if self.filtered.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len() - 1;
        }
    }

    fn recalculate_size(&mut self) {
        let (w, h) = size().unwrap_or((80, 20));
        self.width = w as usize;
        // bash 当前屏不能按整屏填充，否则会把顶部滚走
        self.height = (h as usize).saturating_sub(3).clamp(1, 10);
    }
}

fn run_tui(items: &[String], title: &str) -> Option<String> {
    if terminal::enable_raw_mode().is_err() {
        return Some(items[0].clone());
    }
    let mut out = stdout();

    // 预留足够垂直空间，避免底部截断
    let needed_lines = 13; // prompt + info + help + 最多10行
    for _ in 0..needed_lines {
        writeln!(out).ok();
    }
    execute!(out, MoveUp(needed_lines as u16)).ok();

    let _ = execute!(
        out,
        ResetColor,
        SetAttribute(Attribute::Reset),
        SavePosition,
        Show
    );

    let mut state = State::new(items, title);
    state.recalculate_size();
    render(&mut out, &mut state);

    loop {
        match event::poll(std::time::Duration::from_millis(100)) {
            Ok(true) => {
                if let Ok(Event::Key(KeyEvent {
                    code, modifiers, ..
                })) = event::read()
                {
                    match handle_key(code, modifiers, &mut state) {
                        KeyAction::Select(item) => {
                            cleanup_tui(&mut out, state.total_lines);
                            let _ = terminal::disable_raw_mode();
                            return Some(item);
                        }
                        KeyAction::Quit => {
                            cleanup_tui(&mut out, state.total_lines);
                            let _ = terminal::disable_raw_mode();
                            return None;
                        }
                        KeyAction::Continue => {
                            render(&mut out, &mut state);
                        }
                    }
                }
            }
            Ok(false) => {}
            Err(_) => {
                cleanup_tui(&mut out, state.total_lines);
                let _ = terminal::disable_raw_mode();
                return Some(items[0].clone());
            }
        }
    }
}

enum KeyAction {
    Select(String),
    Quit,
    Continue,
}

struct ResumeState {
    current: Vec<ClaudeSession>,
    all: Vec<ClaudeSession>,
    scope: SessionScope,
    query: String,
    cursor_pos: usize,
    selected: usize,
    offset: usize,
    filtered: Vec<usize>,
    height: usize,
    width: usize,
    total_lines: usize,
    theme: Theme,
}

impl ResumeState {
    fn new(current: &[ClaudeSession], all: &[ClaudeSession]) -> Self {
        let mut state = Self {
            current: current.to_vec(),
            all: all.to_vec(),
            scope: SessionScope::CurrentProject,
            query: String::new(),
            cursor_pos: 0,
            selected: 0,
            offset: 0,
            filtered: Vec::new(),
            height: 0,
            width: 80,
            total_lines: 0,
            theme: Theme::detect(),
        };
        state.update_filtered();
        state
    }

    fn active_sessions(&self) -> &[ClaudeSession] {
        match self.scope {
            SessionScope::CurrentProject => &self.current,
            SessionScope::AllProjects => &self.all,
        }
    }

    fn update_filtered(&mut self) {
        let sessions = self.active_sessions();
        if self.query.is_empty() {
            self.filtered = (0..sessions.len()).collect();
        } else {
            let mut scored = sessions
                .iter()
                .enumerate()
                .filter_map(|(idx, session)| {
                    fuzzy::fuzzy_score(&self.query, &session.search_text).map(|score| (score, idx))
                })
                .collect::<Vec<_>>();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            self.filtered = scored.into_iter().map(|(_, idx)| idx).collect();
        }
        if self.filtered.is_empty() {
            self.selected = 0;
            self.offset = 0;
        } else if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len() - 1;
        }
    }

    fn switch_scope(&mut self, scope: SessionScope) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.selected = 0;
        self.offset = 0;
        self.update_filtered();
    }

    fn selected_session(&self) -> Option<&ClaudeSession> {
        let session_idx = *self.filtered.get(self.selected)?;
        self.active_sessions().get(session_idx)
    }

    fn recalculate_size(&mut self) {
        let (w, h) = size().unwrap_or((100, 24));
        self.width = w as usize;
        self.height = (h as usize).saturating_sub(5).clamp(6, 18);
    }
}

fn run_resume_tui(current: &[ClaudeSession], all: &[ClaudeSession]) -> Option<String> {
    if terminal::enable_raw_mode().is_err() {
        return current
            .first()
            .or_else(|| all.first())
            .map(|session| session.id.clone());
    }
    let mut out = stdout();
    let needed_lines = size()
        .map(|(_, h)| (h as usize).saturating_sub(2).clamp(12, 23))
        .unwrap_or(20);
    for _ in 0..needed_lines {
        writeln!(out).ok();
    }
    execute!(out, MoveUp(needed_lines as u16)).ok();

    let _ = execute!(
        out,
        ResetColor,
        SetAttribute(Attribute::Reset),
        SavePosition,
        Show
    );

    let mut state = ResumeState::new(current, all);
    state.recalculate_size();
    render_resume(&mut out, &mut state);

    loop {
        match event::poll(std::time::Duration::from_millis(100)) {
            Ok(true) => {
                if let Ok(Event::Key(KeyEvent {
                    code, modifiers, ..
                })) = event::read()
                {
                    match handle_resume_key(code, modifiers, &mut state) {
                        KeyAction::Select(item) => {
                            cleanup_tui(&mut out, state.total_lines);
                            let _ = terminal::disable_raw_mode();
                            return Some(item);
                        }
                        KeyAction::Quit => {
                            cleanup_tui(&mut out, state.total_lines);
                            let _ = terminal::disable_raw_mode();
                            return None;
                        }
                        KeyAction::Continue => render_resume(&mut out, &mut state),
                    }
                }
            }
            Ok(false) => {}
            Err(_) => {
                cleanup_tui(&mut out, state.total_lines);
                let _ = terminal::disable_raw_mode();
                return current
                    .first()
                    .or_else(|| all.first())
                    .map(|session| session.id.clone());
            }
        }
    }
}

fn handle_resume_key(code: KeyCode, modifiers: KeyModifiers, state: &mut ResumeState) -> KeyAction {
    match (code, modifiers) {
        (KeyCode::Enter, _) => {
            if let Some(session) = state.selected_session() {
                return KeyAction::Select(session.id.clone());
            }
            KeyAction::Continue
        }
        (KeyCode::Esc, _) => KeyAction::Quit,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => KeyAction::Quit,
        (KeyCode::Left, _) => {
            state.switch_scope(SessionScope::CurrentProject);
            KeyAction::Continue
        }
        (KeyCode::Right, _) => {
            state.switch_scope(SessionScope::AllProjects);
            KeyAction::Continue
        }
        (KeyCode::Backspace, _) => {
            if state.cursor_pos > 0 {
                let idx = state
                    .query
                    .char_indices()
                    .nth(state.cursor_pos - 1)
                    .map(|(i, _)| i)
                    .unwrap_or(state.query.len());
                state.query.remove(idx);
                state.cursor_pos -= 1;
                state.selected = 0;
                state.offset = 0;
                state.update_filtered();
            }
            KeyAction::Continue
        }
        (KeyCode::Delete, _) => {
            if state.cursor_pos < state.query.chars().count() {
                let mut new_query = String::with_capacity(state.query.len());
                for (idx, ch) in state.query.chars().enumerate() {
                    if idx != state.cursor_pos {
                        new_query.push(ch);
                    }
                }
                state.query = new_query;
                state.selected = 0;
                state.offset = 0;
                state.update_filtered();
            }
            KeyAction::Continue
        }
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            if state.cursor_pos > 0 {
                state.cursor_pos -= 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
            if state.cursor_pos < state.query.chars().count() {
                state.cursor_pos += 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
            state.cursor_pos = 0;
            KeyAction::Continue
        }
        (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            state.cursor_pos = state.query.chars().count();
            KeyAction::Continue
        }
        (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            if state.selected > 0 {
                state.selected -= 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
            if state.selected + 1 < state.filtered.len() {
                state.selected += 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Tab, _) => {
            if !state.filtered.is_empty() {
                state.selected = (state.selected + 1) % state.filtered.len();
            }
            KeyAction::Continue
        }
        (KeyCode::BackTab, _) => {
            if state.selected > 0 {
                state.selected -= 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Char(c), _) => {
            let byte_idx = state
                .query
                .char_indices()
                .nth(state.cursor_pos)
                .map(|(i, _)| i)
                .unwrap_or(state.query.len());
            state.query.insert(byte_idx, c);
            state.cursor_pos += 1;
            state.selected = 0;
            state.offset = 0;
            state.update_filtered();
            KeyAction::Continue
        }
        _ => KeyAction::Continue,
    }
}

fn render_resume(out: &mut impl Write, state: &mut ResumeState) {
    state.recalculate_size();
    clear_previous_frame(out);

    let frame_width = state.width.saturating_sub(1);
    let left_width = resume_left_width(frame_width);
    let right_width = frame_width.saturating_sub(left_width).saturating_sub(3);
    let selected = state.selected.min(state.filtered.len().saturating_sub(1));

    if selected < state.offset {
        state.offset = selected;
    }
    if selected >= state.offset + state.height {
        state.offset = selected.saturating_sub(state.height.saturating_sub(1));
    }

    let selected_session = state.selected_session().cloned();
    let preview_lines = selected_session
        .as_ref()
        .map(|session| resume_preview_lines(session, &state.query, state.height))
        .unwrap_or_else(|| vec!["没有匹配会话".to_string()]);

    let theme = &state.theme;
    render_resume_input(out, state, theme);
    render_resume_info(out, state, selected, frame_width, theme);
    render_resume_help(out, frame_width, theme);
    render_resume_header(out, left_width, right_width, theme);

    let mut line_count = 4usize;
    for shown in 0..state.height {
        let filtered_idx = state.offset + shown;
        let session = state
            .filtered
            .get(filtered_idx)
            .and_then(|session_idx| state.active_sessions().get(*session_idx))
            .cloned();
        let preview = preview_lines.get(shown).map(String::as_str).unwrap_or("");
        render_resume_row(
            out,
            session.as_ref(),
            preview,
            &state.query,
            filtered_idx == selected && session.is_some(),
            state.scope,
            left_width,
            right_width,
            theme,
        );
        line_count += 1;
    }

    state.total_lines = line_count;
    let cursor_col = resume_input_cursor_col(state).min(frame_width) as u16;
    let _ = execute!(out, RestorePosition, MoveToColumn(cursor_col), Show);
    out.flush().ok();
}

fn render_resume_input(out: &mut impl Write, state: &ResumeState, theme: &Theme) {
    begin_line(out);
    let prompt = "clash resume>";
    let _ = execute!(out, SetForegroundColor(theme.prompt));
    let _ = write!(out, "{}", prompt);
    reset_style(out);
    let _ = write!(out, " ");
    let _ = execute!(out, SetForegroundColor(theme.text));
    let _ = write!(out, "{}", state.query);
    finish_line(out);
}

fn render_resume_info(
    out: &mut impl Write,
    state: &ResumeState,
    selected: usize,
    frame_width: usize,
    theme: &Theme,
) {
    begin_line(out);
    let filtered = state.filtered.len();
    let current = if filtered == 0 { 0 } else { selected + 1 };
    let scope = match state.scope {
        SessionScope::CurrentProject => "当前项目",
        SessionScope::AllProjects => "全部项目",
    };
    let info = format!("[{scope}] {current}/{filtered}");
    let info_width = display_width(&info);

    let _ = execute!(out, SetForegroundColor(theme.text));
    let _ = write!(out, "{}", info);
    let _ = execute!(
        out,
        ResetColor,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(theme.dim)
    );
    let _ = write!(out, " ");
    draw_rule(
        out,
        frame_width.saturating_sub(info_width).saturating_sub(1),
    );
    reset_style(out);
    finish_line(out);
}

fn render_resume_help(out: &mut impl Write, width: usize, theme: &Theme) {
    begin_line(out);
    let help = "选择会话 | ↑/↓ 选择, ←/→ 切换范围, Enter 确认, Esc 退出";
    let _ = execute!(out, SetForegroundColor(theme.hint));
    write_fit(out, help, width);
    reset_style(out);
    finish_line(out);
}

fn render_resume_header(
    out: &mut impl Write,
    left_width: usize,
    right_width: usize,
    theme: &Theme,
) {
    begin_line(out);
    let _ = execute!(out, SetForegroundColor(theme.dim));
    write_fit(out, "会话", left_width);
    clear_to_width(out, left_width.saturating_sub(display_width("会话")));
    let _ = write!(out, " │ ");
    write_fit(out, "对话历史", right_width);
    reset_style(out);
    finish_line(out);
}

fn render_resume_row(
    out: &mut impl Write,
    session: Option<&ClaudeSession>,
    preview: &str,
    query: &str,
    selected: bool,
    scope: SessionScope,
    left_width: usize,
    right_width: usize,
    theme: &Theme,
) {
    begin_line(out);
    match session {
        Some(session) => {
            apply_row_style(out, selected, theme);
            let marker = if selected { "→ " } else { "  " };
            let _ = write!(out, "{}", marker);
            let used = display_width(marker);
            let label = resume_session_label(session, scope);
            let written = write_fit(out, &label, left_width.saturating_sub(used));
            clear_to_width(out, left_width.saturating_sub(used).saturating_sub(written));
            reset_style(out);
        }
        None => {
            clear_to_width(out, left_width);
        }
    }

    let _ = execute!(out, SetForegroundColor(theme.dim));
    let _ = write!(out, " │ ");
    reset_style(out);
    render_highlighted(out, preview, query, false, right_width, theme);
    reset_style(out);
    finish_line(out);
}

fn resume_left_width(frame_width: usize) -> usize {
    let preferred = frame_width / 2;
    preferred.clamp(32, 50).min(frame_width.saturating_sub(20))
}

fn resume_session_label(session: &ClaudeSession, scope: SessionScope) -> String {
    match scope {
        SessionScope::CurrentProject => session.title.clone(),
        SessionScope::AllProjects => format!("[{}] {}", display_project(session), session.title),
    }
}

fn display_project(session: &ClaudeSession) -> String {
    session
        .cwd
        .as_deref()
        .and_then(|cwd| std::path::Path::new(cwd).file_name())
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| session.project.trim_start_matches('-').to_string())
}

fn resume_preview_lines(session: &ClaudeSession, query: &str, limit: usize) -> Vec<String> {
    if session.preview.is_empty() {
        return vec![format!("{}  {}", session.id, session.path.display())];
    }
    if query.is_empty() {
        return session.preview.iter().take(limit).cloned().collect();
    }

    let query_lower = query.to_lowercase();
    let matched = session
        .preview
        .iter()
        .filter(|line| line.to_lowercase().contains(&query_lower))
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    if matched.is_empty() {
        session.preview.iter().take(limit).cloned().collect()
    } else {
        matched
    }
}

fn resume_input_cursor_col(state: &ResumeState) -> usize {
    let query_prefix: String = state.query.chars().take(state.cursor_pos).collect();
    display_width("clash resume> ") + display_width(&query_prefix)
}

fn handle_key(code: KeyCode, modifiers: KeyModifiers, state: &mut State) -> KeyAction {
    match (code, modifiers) {
        (KeyCode::Enter, _) => {
            if !state.filtered.is_empty() {
                return KeyAction::Select(state.filtered[state.selected].clone());
            }
            KeyAction::Continue
        }
        (KeyCode::Esc, _) => KeyAction::Quit,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => KeyAction::Quit,
        (KeyCode::Backspace, _) => {
            if state.cursor_pos > 0 {
                let idx = state
                    .query
                    .char_indices()
                    .nth(state.cursor_pos - 1)
                    .map(|(i, _)| i)
                    .unwrap_or(state.query.len());
                state.query.remove(idx);
                state.cursor_pos -= 1;
                state.selected = 0;
                state.offset = 0;
                state.update_filtered();
            }
            KeyAction::Continue
        }
        (KeyCode::Delete, _) => {
            if state.cursor_pos < state.query.chars().count() {
                let mut new_q = String::with_capacity(state.query.len());
                for (ci, ch) in state.query.chars().enumerate() {
                    if ci != state.cursor_pos {
                        new_q.push(ch);
                    }
                }
                state.query = new_q;
                state.selected = 0;
                state.offset = 0;
                state.update_filtered();
            }
            KeyAction::Continue
        }
        (KeyCode::Left, _) | (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            if state.cursor_pos > 0 {
                state.cursor_pos -= 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Right, _) | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
            if state.cursor_pos < state.query.chars().count() {
                state.cursor_pos += 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
            state.cursor_pos = 0;
            KeyAction::Continue
        }
        (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            state.cursor_pos = state.query.chars().count();
            KeyAction::Continue
        }
        (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            if state.selected > 0 {
                state.selected -= 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
            if state.selected + 1 < state.filtered.len() {
                state.selected += 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Tab, _) => {
            if !state.filtered.is_empty() {
                state.selected = (state.selected + 1) % state.filtered.len();
            }
            KeyAction::Continue
        }
        (KeyCode::BackTab, _) => {
            if state.selected > 0 {
                state.selected -= 1;
            }
            KeyAction::Continue
        }
        (KeyCode::Char(c), _) => {
            let byte_idx = state
                .query
                .char_indices()
                .nth(state.cursor_pos)
                .map(|(i, _)| i)
                .unwrap_or(state.query.len());
            state.query.insert(byte_idx, c);
            state.cursor_pos += 1;
            state.selected = 0;
            state.offset = 0;
            state.update_filtered();
            KeyAction::Continue
        }
        _ => KeyAction::Continue,
    }
}

fn render(out: &mut impl Write, state: &mut State) {
    state.recalculate_size();

    clear_previous_frame(out);
    let frame_width = state.width.saturating_sub(1);

    let sel = state.selected.min(state.filtered.len().saturating_sub(1));

    if sel < state.offset {
        state.offset = sel;
    }
    if sel >= state.offset + state.height {
        state.offset = sel.saturating_sub(state.height.saturating_sub(1));
    }

    let theme = &state.theme;
    render_input(out, state, theme);
    render_info(out, state, sel, frame_width, theme);
    render_help(out, frame_width, theme, &state.title);

    let mut line_count = 3usize;

    for (shown, i) in (state.offset..state.filtered.len()).enumerate() {
        if shown >= state.height {
            break;
        }

        render_row(
            out,
            &state.filtered[i],
            &state.query,
            i == sel,
            frame_width,
            theme,
            i + 1, // 数字索引
        );
        line_count += 1;
    }

    state.total_lines = line_count;
    let cursor_col = input_cursor_col(state).min(frame_width) as u16;
    let _ = execute!(out, RestorePosition, MoveToColumn(cursor_col), Show);
    out.flush().ok();
}

fn begin_line(out: &mut impl Write) {
    let _ = execute!(out, MoveToColumn(0));
}

fn render_input(out: &mut impl Write, state: &State, theme: &Theme) {
    begin_line(out);
    let prompt = "clash>";
    let _ = execute!(out, SetForegroundColor(theme.prompt));
    let _ = write!(out, "{}", prompt);
    reset_style(out);
    let _ = write!(out, " ");
    let _ = execute!(out, SetForegroundColor(theme.text));
    let _ = write!(out, "{}", state.query);
    finish_line(out);
}

fn render_info(
    out: &mut impl Write,
    state: &State,
    selected: usize,
    frame_width: usize,
    theme: &Theme,
) {
    begin_line(out);
    let filtered = state.filtered.len();
    let current = if filtered == 0 { 0 } else { selected + 1 };
    let count = format!("{current}/{filtered}");
    let count_width = display_width(&count);

    let _ = execute!(out, SetForegroundColor(theme.text));
    let _ = write!(out, "{}", count);
    let _ = execute!(
        out,
        ResetColor,
        SetAttribute(Attribute::Reset),
        SetForegroundColor(theme.dim)
    );
    let _ = write!(out, " ");
    draw_rule(
        out,
        frame_width.saturating_sub(count_width).saturating_sub(1),
    );
    reset_style(out);
    finish_line(out);
}

fn render_help(out: &mut impl Write, width: usize, theme: &Theme, title: &str) {
    begin_line(out);
    let help = format!("{} | ↑/↓ 选择, Enter 确认, Esc 退出", title);
    let _ = execute!(out, SetForegroundColor(theme.hint));
    write_fit(out, &help, width);
    reset_style(out);
    finish_line(out);
}

fn render_row(
    out: &mut impl Write,
    item: &str,
    query: &str,
    selected: bool,
    width: usize,
    theme: &Theme,
    index: usize,
) {
    begin_line(out);
    let prefix = format!("{}. ", index); // "1. " "2. "
    let used = MARKER_END_COL as usize + display_width(&prefix);
    let item_width = width.saturating_sub(used);

    apply_row_style(out, selected, theme);
    if selected {
        let _ = execute!(out, SetForegroundColor(theme.select));
        let _ = write!(out, "→ ");
        apply_row_style(out, true, theme);
    } else {
        let _ = write!(out, "  ");
    }
    let _ = execute!(out, MoveToColumn(MARKER_END_COL));
    let _ = write!(out, "{}", prefix);
    let rendered = render_highlighted(out, item, query, selected, item_width, theme);
    apply_row_style(out, selected, theme);
    clear_to_width(out, item_width.saturating_sub(rendered));
    reset_style(out);
    finish_line(out);
}

fn apply_row_style(out: &mut impl Write, selected: bool, theme: &Theme) {
    if selected {
        let _ = execute!(
            out,
            SetBackgroundColor(theme.select_bg),
            SetForegroundColor(theme.text)
        );
    } else {
        let _ = execute!(
            out,
            ResetColor,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(theme.text)
        );
    }
}

/// 用 fzf 风格高亮匹配字符，同时严格限制显示宽度
fn render_highlighted(
    out: &mut impl Write,
    text: &str,
    query: &str,
    selected: bool,
    max_width: usize,
    theme: &Theme,
) -> usize {
    if query.is_empty() {
        apply_row_style(out, selected, theme);
        return write_fit(out, text, max_width);
    }

    let text_lower = text.to_lowercase();
    let query_lower = query.to_lowercase();
    let query_chars: Vec<char> = query_lower.chars().collect();

    let mut matched = vec![false; text.chars().count()];
    let mut search_from = 0usize;
    let text_chars: Vec<char> = text_lower.chars().collect();

    for &qc in &query_chars {
        if let Some(pos) = text_chars[search_from..].iter().position(|&c| c == qc) {
            let actual = search_from + pos;
            matched[actual] = true;
            search_from = actual + 1;
        } else {
            break;
        }
    }

    let mut in_match = false;
    let mut written = 0usize;

    for (i, ch) in text.chars().enumerate() {
        let ch_width = ch.width().unwrap_or(0);
        if written + ch_width > max_width {
            break;
        }

        let is_match = matched[i];
        if is_match && !in_match {
            set_match_style(out, selected, theme);
            in_match = true;
        } else if !is_match && in_match {
            apply_row_style(out, selected, theme);
            in_match = false;
        }
        let _ = write!(out, "{}", ch);
        written += ch_width;
    }

    if in_match {
        apply_row_style(out, selected, theme);
    }

    written
}

fn set_match_style(out: &mut impl Write, selected: bool, theme: &Theme) {
    if selected {
        let _ = execute!(
            out,
            SetBackgroundColor(theme.select_bg),
            SetForegroundColor(theme.match_color)
        );
    } else {
        let _ = execute!(out, SetForegroundColor(theme.match_color));
    }
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn input_cursor_col(state: &State) -> usize {
    let query_prefix: String = state.query.chars().take(state.cursor_pos).collect();
    display_width("clash> ") + display_width(&query_prefix)
}

fn write_fit(out: &mut impl Write, text: &str, max_width: usize) -> usize {
    let mut written = 0usize;

    for ch in text.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if written + ch_width > max_width {
            break;
        }
        let _ = write!(out, "{}", ch);
        written += ch_width;
    }

    written
}

fn clear_to_width(out: &mut impl Write, width: usize) {
    if width == 0 {
        return;
    }
    let _ = write!(out, "{}", " ".repeat(width));
}

/// 清掉行尾旧字符，避免切换选中时残留
fn finish_line(out: &mut impl Write) {
    let _ = execute!(out, Clear(ClearType::UntilNewLine));
    writeln!(out).ok();
}

fn draw_rule(out: &mut impl Write, width: usize) {
    if width == 0 {
        return;
    }
    let _ = write!(out, "{}", "-".repeat(width));
}

fn reset_style(out: &mut impl Write) {
    let _ = execute!(out, ResetColor, SetAttribute(Attribute::Reset));
}

fn clear_previous_frame(out: &mut impl Write) {
    let _ = execute!(out, ResetColor, SetAttribute(Attribute::Reset));
    let _ = execute!(
        out,
        RestorePosition,
        MoveToColumn(0),
        Clear(ClearType::FromCursorDown)
    );
}

fn cleanup_tui(out: &mut impl Write, total_lines: usize) {
    let _ = total_lines;
    clear_previous_frame(out);
    let _ = execute!(out, ResetColor, SetAttribute(Attribute::Reset), Show);
    let _ = out.flush();
}
