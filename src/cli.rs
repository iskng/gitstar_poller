use clap::Parser;

#[derive(Parser)]
#[command(name = "github-stars-server")]
#[command(
    about = "GitHub Stars Processing Server - Processes repository stargazers in the background"
)]
#[command(version = "0.1.0")]
pub struct Cli {
    /// Use local development settings (sets DB URL to ws://localhost:8000)
    #[arg(long)]
    pub local: bool,

    /// SurrealDB connection URL
    #[arg(long, env = "DB_URL", default_value = "ws://localhost:8000")]
    pub db_url: String,

    /// SurrealDB username
    #[arg(long, env = "DB_USER", default_value = "root")]
    pub db_user: String,

    /// SurrealDB password
    #[arg(long, env = "DB_PASS", default_value = "root")]
    pub db_pass: String,

    /// SurrealDB namespace
    #[arg(long, env = "DB_NAMESPACE", default_value = "gitstars")]
    pub db_namespace: String,

    /// SurrealDB database
    #[arg(long, env = "DB_DATABASE", default_value = "stars")]
    pub db_database: String,

    /// Maximum size of the database connection pool
    #[arg(long, env = "DB_POOL_MAX_SIZE", default_value = "10")]
    pub db_pool_max_size: usize,

    /// Minimum idle connections in the pool
    #[arg(long, env = "DB_POOL_MIN_IDLE", default_value = "2")]
    pub db_pool_min_idle: usize,

    /// Connection timeout in seconds
    #[arg(long, env = "DB_CONNECTION_TIMEOUT", default_value = "30")]
    pub db_connection_timeout: u64,
}
