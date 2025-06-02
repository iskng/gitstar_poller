use github_stars_server::surreal_client::SurrealClient;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct TestContext {
    pub db_client: Arc<Mutex<SurrealClient>>,
}

impl TestContext {
    pub async fn new() -> anyhow::Result<Self> {
        // Use environment variables or defaults for dev database
        let db_url = std::env::var("DB_URL").unwrap_or_else(|_| "ws://localhost:8000".to_string());
        let db_user = std::env::var("DB_USER").unwrap_or_else(|_| "root".to_string());
        let db_pass = std::env::var("DB_PASS").unwrap_or_else(|_| "root".to_string());
        let db_namespace = std::env::var("DB_NAMESPACE").unwrap_or_else(|_| "gitstars".to_string());
        let db_database = std::env::var("DB_DATABASE").unwrap_or_else(|_| "stars".to_string());

        // Try to connect with a timeout
        match
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                SurrealClient::new(&db_url, &db_user, &db_pass, &db_namespace, &db_database)
            ).await
        {
            Ok(Ok(db_client)) => {
                Ok(TestContext {
                    db_client: Arc::new(Mutex::new(db_client)),
                })
            }
            Ok(Err(e)) => {
                eprintln!("Failed to connect to SurrealDB: {}", e);
                eprintln!("Make sure SurrealDB is running with file backend, not memory backend");
                eprintln!(
                    "Example: surreal start --bind 0.0.0.0:8000 --user root --pass root file://./test.db --allow-all"
                );
                Err(e)
            }
            Err(_) => {
                let err = anyhow::anyhow!(
                    "Connection to SurrealDB timed out. WebSocket connections may not work with memory backend."
                );
                eprintln!("{}", err);
                eprintln!(
                    "Current SurrealDB may be running with memory backend which doesn't support WebSocket connections properly."
                );
                eprintln!("To fix: restart SurrealDB with file backend instead of memory backend");
                Err(err)
            }
        }
    }
}
