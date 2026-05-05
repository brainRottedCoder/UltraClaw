use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use std::{error::Error, io};

const CAPABILITIES: &[&str] = &[
    // OS Nodes
    "camera_snap", "camera_clip", "screen_record", "location_get",
    "system_run", "system_notify", "sessions_list", "sessions_history", "sessions_send", "sessions_spawn",
    // Channels
    "Slack", "Discord", "Telegram", "WhatsApp", "Google Chat", "Signal", "BlueBubbles", "iMessage",
    "Microsoft Teams", "Zalo", "ZaloPersonal", "WebChat", "Feishu", "QQ", "DingTalk", "LINE", "WeCom", "Nostr", "Twitch", "Mattermost",
    // OpenClaw Skills (abridged for TUI)
    "1Password", "GitHub_Issues", "GitHub_PRs", "Notion", "Trello", "Asana", "Bear_Notes", "Apple_Notes", "Obsidian", "Jira", "Linear", "Docker_Manage", "Kubernetes_Pods", "AWS_EC2", "Tailscale_Serve", "Canvas_Render", "Git_Conflict_Resolve", "Apple_Health", "Sonos_Speaker", "HomeAssistant", "Hue_Lights", "Browser_Puppeteer", "Google_Drive", "Gmail", "Calendar", "Zoom_Launch", "Stripe_Charge", "Spotify_Play", "Figma_Read", "Slack_Post",
];

pub fn run_tui() -> Result<(), Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let reverse_caps: Vec<&str> = CAPABILITIES.iter().rev().copied().collect();
    let mut list_state = ListState::default();
    list_state.select(Some(0));

    let res = run_app(&mut terminal, &reverse_caps, &mut list_state);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    caps: &[&str],
    state: &mut ListState,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .margin(1)
                .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
                .split(f.area());

            let items: Vec<ListItem> = caps
                .iter()
                .map(|i| {
                    ListItem::new(Line::from(Span::styled(
                        *i,
                        Style::default().fg(Color::Cyan),
                    )))
                })
                .collect();

            let capabilities_list = List::new(items)
                .block(Block::default().title(" Universal Capabilities (Bottom->Top) ").borders(Borders::ALL))
                .style(Style::default().fg(Color::White))
                .highlight_style(Style::default().add_modifier(Modifier::ITALIC).bg(Color::DarkGray))
                .highlight_symbol(">> ");
            
            f.render_stateful_widget(capabilities_list, chunks[0], state);

            let selected_idx = state.selected().unwrap_or(0);
            let selected_name = caps[selected_idx];
            let desc = format!("{} active and standing by.\n\nPress <ENTER> to trigger this capability.\nPress <q> to quit the Ultraclaw Terminal Interface.", selected_name);

            let info_block = Paragraph::new(desc)
                .block(Block::default().title(" Capability Details ").borders(Borders::ALL))
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(Color::Yellow));

            f.render_widget(info_block, chunks[1]);
        })?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Down => {
                    let i = match state.selected() {
                        Some(i) => {
                            if i >= caps.len() - 1 {
                                0
                            } else {
                                i + 1
                            }
                        }
                        None => 0,
                    };
                    state.select(Some(i));
                }
                KeyCode::Up => {
                    let i = match state.selected() {
                        Some(i) => {
                            if i == 0 {
                                caps.len() - 1
                            } else {
                                i - 1
                            }
                        }
                        None => 0,
                    };
                    state.select(Some(i));
                }
                _ => {}
            }
        }
    }
}
