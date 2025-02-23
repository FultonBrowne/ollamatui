use crossterm::{
    cursor::EnableBlinking,
    event::{self, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::stream::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Position},
    text::Text,
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::{io, time::Duration};
use tokio::runtime::Runtime;

#[derive(Serialize, Deserialize, Debug)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct ChatHistory {
    messages: Vec<Message>,
}

impl ChatHistory {
    fn clone(&self) -> ChatHistory {
        ChatHistory {
            messages: self
                .messages
                .iter()
                .map(|m| Message {
                    role: m.role.clone(),
                    content: m.content.clone(),
                })
                .collect(),
        }
    }
}

const API_URL: &str = "http://localhost:11434/api/chat";

async fn send_message(
    client: &Client,
    chat_history: &ChatHistory,
    model: &str,
    tx: Sender<String>,
) {
    let response = client
        .post(API_URL)
        .json(&serde_json::json!({
            "model": model,
            "messages": chat_history.messages,
            "stream": true // Enable streaming
        }))
        .send()
        .await;

    if let Ok(resp) = response {
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            if let Ok(bytes) = chunk {
                if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                    if let Ok(json_value) = serde_json::from_str::<Value>(&text) {
                        if let Some(content) = json_value["message"]["content"].as_str() {
                            tx.send(content.to_string()).unwrap();
                        }
                    }
                }
            }
        }
    }
}

fn main() -> Result<(), io::Error> {
    // Read command-line arguments
    let args: Vec<String> = env::args().collect();
    let model = if args.len() > 1 { &args[1] } else { "llama3.2" };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBlinking)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut input = String::new();
    let mut chat_history = ChatHistory { messages: vec![] };
    let mut scroll_offset = 0;

    let client = Client::new();

    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
                .split(f.area());

            let history_text = chat_history
                .messages
                .iter()
                .map(|m| format!("{}: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n");

            let lines: Vec<_> = history_text.lines().collect();
            let total_lines = lines.len();
            let display_start = scroll_offset.min(total_lines);
            let display_end = total_lines;
            let displayed_text = lines[display_start..display_end].join("\n");

            let history_paragraph = Paragraph::new(Text::from(displayed_text))
                .block(Block::default().borders(Borders::ALL).title("Chat History"))
                .wrap(Wrap { trim: true });
            let input_paragraph = Paragraph::new(input.as_str())
                .block(Block::default().borders(Borders::ALL).title("Input"));

            f.render_widget(history_paragraph, chunks[0]);
            f.render_widget(input_paragraph, chunks[1]);

            // Set the cursor position to the end of the input text
            let input_area = chunks[1];
            let cursor_x = input_area.x + input.len() as u16 + 1;
            let cursor_y = input_area.y + 1;
            f.set_cursor_position(Position {
                x: cursor_x,
                y: cursor_y,
            });
        })?;

        // Check for streaming updates
        while let Ok(content) = rx.try_recv() {
            if let Some(last_message) = chat_history.messages.last_mut() {
                if last_message.role == "assistant" {
                    last_message.content.push_str(&content);
                }
            }
        }

        if event::poll(Duration::from_millis(100))? {
            if let event::Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Enter => {
                        chat_history.messages.push(Message {
                            role: "user".to_string(),
                            content: input.clone(),
                        });

                        // Start streaming response
                        let assistant_message = Message {
                            role: "assistant".to_string(),
                            content: String::new(),
                        };
                        chat_history.messages.push(assistant_message);

                        let client_clone = client.clone();
                        let chat_history_clone = chat_history.clone();
                        let tx_clone = tx.clone();
                        let model_clone = model.to_string();

                        thread::spawn(move || {
                            let runtime = Runtime::new().unwrap();
                            runtime.block_on(send_message(
                                &client_clone,
                                &chat_history_clone,
                                &model_clone,
                                tx_clone,
                            ));
                        });

                        input.clear();
                    }
                    KeyCode::Char(c) => input.push(c),
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Esc => break,
                    KeyCode::PageUp => {
                        if scroll_offset > 0 {
                            scroll_offset -= 5;
                        }
                    }
                    KeyCode::PageDown => {
                        scroll_offset += 5;
                    }
                    _ => {}
                }
            }
        }
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
