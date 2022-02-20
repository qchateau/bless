use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::error;
use std::io;

pub struct ConfigureTerm {
    is_cleanup: bool,
}

impl ConfigureTerm {
    pub fn new() -> io::Result<ConfigureTerm> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        return Ok(ConfigureTerm { is_cleanup: false });
    }

    pub fn cleanup(&mut self) {
        if self.is_cleanup {
            return;
        }
        // try to reset befor the panic to get the info properly
        disable_raw_mode().unwrap_or_else(|e| error!("error disabling raw mode: {}", e));
        execute!(io::stdout(), LeaveAlternateScreen)
            .unwrap_or_else(|e| error!("error leaving alternate screen: {}", e));
        self.is_cleanup = true;
    }
}

impl Drop for ConfigureTerm {
    fn drop(&mut self) {
        self.cleanup()
    }
}
