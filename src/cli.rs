use clap::Parser;

#[derive(Parser)]
#[command(name = "github-stars-server")]
#[command(about = "GitHub Stars Processing Server - Processes repository stargazers in the background")]
#[command(version = "0.1.0")]
pub struct Cli {
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
}