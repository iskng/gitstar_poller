# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

- `cargo run` - Run the server with default settings
- `cargo run -- --local` - Run in local development mode (automatically uses ws://localhost:8000)
- `cargo run -- --help` - Show all available CLI options
- `cargo build` - Build the application in debug mode
- `cargo build --release` - Build optimized release version
- `cargo test` - Run all tests
- `cargo check` - Check code without building
- `cargo clippy` - Run Rust linter for code quality
- `cargo fmt` - Format code according to Rust standards

## Architecture Overview

This is a GitHub Stars Processing Server that analyzes repository stargazers in the background using actor-based concurrency with SurrealDB for persistence.

### Project Structure

- `src/main.rs` - Entry point and server initialization
- `src/cli.rs` - Command line argument parsing with clap
- `src/github.rs` - GitHub API client with rate limiting and pagination
- `src/models.rs` - Data models for SurrealDB integration
- `src/surreal_client.rs` - SurrealDB client with live query support
- `src/pool.rs` - Deadpool connection pool implementation for SurrealDB
- `src/error.rs` - Error handling and custom error types
- `src/actors/` - Actor system components using ractor
  - `processing_supervisor.rs` - Top-level supervisor managing the entire system
  - `github_factory.rs` - Factory for creating and managing GitHub workers
  - `github_worker.rs` - Worker actors that process repository stargazers
- `src/types.rs` - Type definitions (legacy from CLI mode)

### Key Features

- **Actor-Based Concurrency**: Uses ractor for scalable, fault-tolerant processing
- **Connection Pooling**: Implements deadpool for efficient database connection management
- **GitHub API Integration**: Fetches stargazers with automatic rate limiting
- **Live Queries**: Reacts to new GitHub accounts in real-time via SurrealDB
- **Automatic Retry**: Handles rate limits and failures gracefully
- **Sticky Routing**: Ensures same user's repos go to same worker for optimal token usage

### Database Connection Pooling

The server uses deadpool to manage SurrealDB connections efficiently:

- **Pool Configuration**:
  - `--db-pool-max-size` (default: 10) - Maximum connections in the pool
  - `--db-pool-min-idle` (default: 2) - Minimum idle connections to maintain
  - `--db-connection-timeout` (default: 30s) - Timeout for acquiring connections

- **Architecture**:
  - Pool created at supervisor level and shared via `Arc<SurrealPool>`
  - Workers get connections per job (not per worker lifetime)
  - Connections automatically returned to pool when dropped
  - Health checks ensure connection validity

### Environment Setup

1. Create a `.env` file with the following variables:

```env
# SurrealDB Configuration
DB_URL=ws://localhost:8000
DB_USER=root
DB_PASS=root
DB_NAMESPACE=gitstars  # Must match Next.js app
DB_DATABASE=stars       # Must match Next.js app

# Connection Pool Settings (optional)
DB_POOL_MAX_SIZE=10
DB_POOL_MIN_IDLE=2
DB_CONNECTION_TIMEOUT=30
```

2. Start SurrealDB (if not already running):
```bash
surreal start --bind 0.0.0.0:8000 --user root --pass root --log info memory --allow-all
```

### Usage Examples

```bash
# Run with default settings (reads from .env)
cargo run

# Run in local development mode
cargo run -- --local

# Run with custom database settings
cargo run -- --db-url ws://localhost:8000 --db-user root --db-pass root

# Run with custom pool configuration
cargo run -- --db-pool-max-size 20 --db-pool-min-idle 5

# Show all available options
cargo run -- --help
```

### CLI Options

- `--local` - Use local development settings (DB URL: ws://localhost:8000)
- `--db-url <URL>` - SurrealDB connection URL (default: ws://localhost:8000)
- `--db-user <USER>` - Database username (default: root)
- `--db-pass <PASS>` - Database password (default: root)
- `--db-namespace <NS>` - Database namespace (default: gitstars)
- `--db-database <DB>` - Database name (default: stars)
- `--db-pool-max-size <N>` - Maximum pool connections (default: 10)
- `--db-pool-min-idle <N>` - Minimum idle connections (default: 2)
- `--db-connection-timeout <SECS>` - Connection timeout in seconds (default: 30)

### Server Architecture

1. **ProcessingSupervisor** (Top Level):
   - Creates and manages the connection pool
   - Sets up live queries for new GitHub accounts
   - Spawns and manages the GitHubFactory
   - Handles graceful shutdown
   - Provides system-wide statistics

2. **GitHubFactory** (Factory Pattern):
   - Manages a pool of GitHubWorker actors
   - Routes jobs using sticky routing (same user → same worker)
   - Implements dead man's switch for stuck workers
   - Handles job queue management

3. **GitHubWorker** (Workers):
   - Processes individual repository stargazer fetching jobs
   - Gets database connections from pool per job
   - Handles GitHub API rate limiting
   - Claims repos atomically to prevent duplicate work
   - Inserts stargazers in batches for efficiency

### Workflow

1. **Startup**:
   - Creates connection pool with configured size
   - Connects to SurrealDB and sets up live query
   - Spawns supervisor → factory → workers
   - Processes any existing accounts

2. **New Account Detection**:
   - Live query detects new GitHub account
   - Waits for repo sync to stabilize
   - Submits jobs for each starred repo
   - Jobs routed to workers via sticky routing

3. **Repository Processing**:
   - Worker claims repo (atomic operation)
   - Fetches stargazers page by page
   - Inserts github_users and relationships in batches
   - Updates progress after each page
   - Marks repo complete or failed

4. **Error Handling**:
   - Rate limits: Worker waits until reset time
   - API errors: Repo marked as failed
   - Connection errors: Automatic retry from pool
   - Worker crashes: Dead man's switch restarts

### Integration with Next.js Frontend

The server integrates with a Next.js application that:
1. Handles user authentication with GitHub
2. Syncs user's starred repositories
3. Creates account and repo records in SurrealDB
4. The server detects these via live queries and processes stargazers

### Performance Considerations

- **Connection Pool**: Scales database operations with worker count
- **Batch Operations**: Reduces database round trips
- **Sticky Routing**: Optimizes GitHub API token usage
- **Rate Limit Handling**: Prevents API quota exhaustion
- **Concurrent Processing**: Multiple workers process different repos in parallel

### Monitoring and Debugging

- **Logging**: Uses tracing for structured logging
- **Statistics**: Get runtime stats via supervisor
- **Health Checks**: Connection pool validates connections
- **Graceful Shutdown**: Ctrl+C triggers clean shutdown with final stats

### Dependencies

Key dependencies include:
- `ractor` - Actor framework for concurrent processing
- `deadpool` - Connection pooling with Tokio runtime support
- `surrealdb` - Database client with WebSocket support
- `tokio` - Async runtime
- `reqwest` - HTTP client for GitHub API
- `clap` - Command line parsing
- `tracing` - Structured logging