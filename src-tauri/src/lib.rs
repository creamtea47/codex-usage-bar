mod app;
mod domain;
mod infra;
mod ipc;

pub use app::bootstrap::run;

#[cfg(test)]
mod test_support;
