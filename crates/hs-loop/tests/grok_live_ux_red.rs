//! RED-first contracts from side-by-side PTY runs of grok-cli fb97af83 and HAIRSPRING.
use hs_loop::tui::{self, KeyAction, TuiState};
use ratatui::{Terminal, backend::TestBackend};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
fn screen(st: &TuiState, w: u16, h: u16) -> String {
 let mut t=Terminal::new(TestBackend::new(w,h)).unwrap();
 t.draw(|f| tui::render_skeleton(f,st)).unwrap(); let b=t.backend().buffer();
 (0..h).map(|y|(0..w).map(|x|b[(x,y)].symbol()).collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n")
}
fn type_text(st:&mut TuiState,s:&str){ for c in s.chars(){ assert_eq!(tui::handle_key(st,KeyEvent::new(KeyCode::Char(c),KeyModifiers::NONE)),KeyAction::Continue); } }
#[test]
fn slash_help_opens_searchable_overlay_instead_of_appending_a_wall_of_text(){
 let mut st=TuiState::default(); type_text(&mut st,"/help");
 let action=tui::handle_key(&mut st,KeyEvent::new(KeyCode::Enter,KeyModifiers::NONE));
 assert_eq!(action,KeyAction::Continue,"/help is a UI transition, not bin transcript output");
 let s=screen(&st,100,32);
 assert!(s.contains("Commands") && s.contains("Search..."),"grok live /help modal hierarchy: {s}");
 assert!(st.picker.is_some() || st.palette.is_some(),"help must remain keyboard navigable and escape-dismissable");
}
#[test]
fn escape_interrupts_an_active_run_like_the_live_reference(){
 let mut st=TuiState::default(); st.phase=tui::LoopPhase::Act;
 assert_eq!(tui::handle_key(&mut st,KeyEvent::new(KeyCode::Esc,KeyModifiers::NONE)),KeyAction::Interrupt);
}
