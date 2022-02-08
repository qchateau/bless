use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gag::Redirect;
use std::{fs, io};

pub struct ConfigureTerm {
    is_cleanup: bool,
    stderr_redirection: Option<Redirect<fs::File>>,
}

impl ConfigureTerm {
    pub fn new() -> io::Result<ConfigureTerm> {
        let stderr_redirection = if atty::is(atty::Stream::Stderr) {
            Some(
                Redirect::stderr(
                    fs::OpenOptions::new()
                        .truncate(true)
                        .read(true)
                        .create(true)
                        .write(true)
                        .open("/dev/null")
                        .unwrap(),
                )
                .unwrap(),
            )
        } else {
            None
        };

        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;

        return Ok(ConfigureTerm {
            is_cleanup: false,
            stderr_redirection,
        });
    }

    pub fn cleanup(&mut self) {
        if self.is_cleanup {
            return;
        }
        // try to reset befor the panic to get the info properly
        disable_raw_mode().unwrap_or_else(|e| eprintln!("error disabling raw mode: {}", e));
        execute!(io::stdout(), LeaveAlternateScreen)
            .unwrap_or_else(|e| eprintln!("error leaving alternate screen: {}", e));
        self.stderr_redirection.take();
        self.is_cleanup = true;
    }
}

impl Drop for ConfigureTerm {
    fn drop(&mut self) {
        self.cleanup()
    }
}
