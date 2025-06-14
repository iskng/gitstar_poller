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
- `src/github.rs` - GitHub API client with rate limiting and pagination limit detection
- `src/models.rs` - Data models for SurrealDB integration (includes ProcessingStatus enum with PaginationLimited)
- `src/surreal_client.rs` - SurrealDB client with live query support and typed ProcessingStats struct
- `src/pool.rs` - Deadpool connection pool implementation for SurrealDB
- `src/error.rs` - Error handling and custom error types (includes PaginationLimitExceeded)
- `src/actors/` - Actor system components using ractor
  - `processing_supervisor.rs` - Top-level supervisor managing the entire system with hourly diagnostics
  - `github_factory.rs` - Factory for creating and managing GitHub workers with priority queue
  - `github_worker.rs` - Worker actors that process repository stargazers with health monitoring
- `src/types.rs` - Type definitions (legacy from CLI mode)

### Key Features

- **Actor-Based Concurrency**: Uses ractor for scalable, fault-tolerant processing
- **Connection Pooling**: Implements deadpool for efficient database connection management
- **GitHub API Integration**: Fetches stargazers with automatic rate limiting and pagination limit handling
- **Live Queries**: Reacts to new GitHub accounts in real-time via SurrealDB
- **Automatic Retry**: Handles rate limits, transaction conflicts, and failures gracefully
- **Sticky Routing**: Ensures same user's repos go to same worker for optimal token usage
- **Dynamic Worker Scaling**: Automatically scales workers based on queue depth and system resources (max 1000)
- **Pagination Limit Handling**: Detects GitHub's 400-page (~40,000 stars) limit and marks repos appropriately
- **Priority Queue**: New accounts get high priority, existing accounts get low priority
- **Hourly Statistics**: Reports comprehensive system metrics every hour
- **Dead Man's Switch**: Kills stuck workers after 6 hours (configurable)
- **Transaction Conflict Retry**: Automatically retries database operations on conflicts

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

# Health Check Server (optional)
HEALTH_PORT=8080  # Set to 0 to disable

# Admin API Server (optional)
ADMIN_PORT=8081   # Set to 0 to disable
ADMIN_API_KEY=your-secret-api-key  # Required if admin port is enabled
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

# Run with admin API enabled
cargo run -- --admin-port 8081
# Note: Set ADMIN_API_KEY environment variable first

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
- `--health-port <PORT>` - Health check server port (default: 8080, set to 0 to disable)
- `--admin-port <PORT>` - Admin API server port (default: 8081, set to 0 to disable)
- `--admin-api-key <KEY>` - Admin API authentication key (required if admin port is enabled)

### Server Architecture

1. **ProcessingSupervisor** (Top Level):
   - Creates and manages the connection pool
   - Sets up live queries for new GitHub accounts
   - Spawns and manages the GitHubFactory
   - Handles graceful shutdown
   - Provides system-wide statistics
   - Reports hourly diagnostics with detailed metrics
   - Monitors system resources (CPU and memory)
   - Dynamically scales workers based on load

2. **GitHubFactory** (Factory Pattern):
   - Manages a pool of GitHubWorker actors
   - Routes jobs using sticky routing (same user → same worker)
   - Implements dead man's switch for stuck workers (6 hour timeout)
   - Handles job queue management with priority levels
   - Priority queue: High priority for new accounts, Low for existing

3. **GitHubWorker** (Workers):
   - Processes individual repository stargazer fetching jobs
   - Gets database connections from pool per job
   - Handles GitHub API rate limiting with intelligent delays
   - Claims repos atomically to prevent duplicate work
   - Inserts stargazers in batches for efficiency
   - Tracks processing state for monitoring
   - Unclaims repos on shutdown/error
   - Sorts repos by star count (processes smallest first)

### Workflow

1. **Startup**:
   - Creates connection pool with configured size
   - Connects to SurrealDB and sets up live query
   - Spawns supervisor → factory → workers
   - Initial worker count based on number of existing accounts
   - Processes any existing accounts with low priority

2. **New Account Detection**:
   - Live query detects new GitHub account
   - Waits for repo sync to stabilize (5 seconds of stable count)
   - Submits jobs for each starred repo with high priority
   - Jobs routed to workers via sticky routing

3. **Repository Processing**:
   - Worker claims repo (atomic operation with 1-hour timeout)
   - Fetches stargazers page by page (100 per page)
   - Inserts github_users and relationships in batches
   - Updates progress after each page
   - Marks repo complete, failed, or pagination_limited
   - Logs progress every 10 pages for large repos

4. **Error Handling**:
   - Rate limits: Worker waits until reset time
   - Pagination limits: Repo marked as pagination_limited (not an error)
   - API errors: Repo marked as failed with error message
   - Connection errors: Automatic retry from pool
   - Transaction conflicts: Retry up to 3 times with backoff
   - Worker crashes: Dead man's switch restarts after 6 hours
   - Stale claims: Reset after 60 minutes by cleanup task

### Integration with Next.js Frontend

The server integrates with a Next.js application that:
1. Handles user authentication with GitHub
2. Syncs user's starred repositories (100 at a time)
3. Creates account and repo records in SurrealDB
4. The server detects these via live queries and processes stargazers

### Admin API

The admin API provides authenticated access to system management functions:

#### Authentication
All admin API endpoints require Bearer token authentication:
```bash
curl -H "Authorization: Bearer your-secret-api-key" http://localhost:8081/stats
```

#### Endpoints

**Statistics:**
- `GET /stats` - Detailed statistics including processing, database, and queue info
- `GET /stats/summary` - Summary statistics from the processing supervisor

**Repository Management:**
- `GET /repos?status=failed&limit=100&offset=0` - List repositories with optional filtering
- `GET /repos/:repo_id` - Get specific repository processing status
- `POST /repos/:repo_id/reprocess` - Reprocess a repository (optionally with `{"force": true}`)
- `POST /repos/:repo_id/reset` - Reset repository processing status

**Worker Management:**
- `GET /workers` - Get worker information (active, idle, queue depth, resources)
- `POST /workers/scale` - Scale workers (not yet implemented)

**Queue Management:**
- `GET /queue` - Get queue information
- `POST /queue/clear` - Clear the queue (not yet implemented)

**User Management:**
- `POST /users/:user_id/reprocess` - Reprocess all repositories for a user

**System Control:**
- `POST /pause` - Pause processing (not yet implemented)
- `POST /resume` - Resume processing (not yet implemented)

#### Example Usage
```bash
# Get system stats
curl -H "Authorization: Bearer your-secret-api-key" http://localhost:8081/stats

# List failed repos
curl -H "Authorization: Bearer your-secret-api-key" http://localhost:8081/repos?status=failed

# Reprocess a specific repo
curl -X POST -H "Authorization: Bearer your-secret-api-key" \
  -H "Content-Type: application/json" \
  -d '{"force": true}' \
  http://localhost:8081/repos/123456/reprocess

# Get worker information
curl -H "Authorization: Bearer your-secret-api-key" http://localhost:8081/workers
```

### Performance Considerations

- **Connection Pool**: Scales database operations with worker count
- **Batch Operations**: Reduces database round trips
- **Sticky Routing**: Optimizes GitHub API token usage
- **Rate Limit Handling**: Prevents API quota exhaustion (targets 4000 req/hour)
- **Concurrent Processing**: Multiple workers process different repos in parallel
- **Worker Scaling**: Dynamically adjusts workers based on queue depth and system resources
- **Intelligent Delays**: Increases delay when rate limit is low
- **Resource Monitoring**: Prevents scaling when CPU > 80% or Memory > 85%

### Monitoring and Debugging

- **Logging**: Uses tracing for structured logging
  - Default: `warn,github_stars_server=info` (shows warnings from all crates, info from our crate)
  - Hourly diagnostic reports at INFO level
  - Worker progress updates every 10 pages
  - Override with RUST_LOG environment variable
- **Hourly Diagnostics**: Reports processing stats, worker status, system resources, and database metrics
- **Health Checks**: 
  - HTTP health check server on port 8080 (configurable)
  - `/health` or `/healthz` - Comprehensive health status with statistics
  - `/livez` - Kubernetes liveness probe (simple alive check)
  - `/readyz` - Kubernetes readiness probe (checks DB and supervisor)
  - Returns 503 when unhealthy, 200 when healthy/degraded
- **Admin API**: Authenticated REST API for system management (port 8081)
  - `/stats` - Get detailed system statistics
  - `/stats/summary` - Get summary statistics
  - `/repos` - List repositories with filtering
  - `/repos/:id` - Get specific repository status
  - `/repos/:id/reprocess` - Reprocess a repository
  - `/repos/:id/reset` - Reset repository processing status
  - `/workers` - Get worker information
  - `/queue` - Get queue information
  - `/users/:id/reprocess` - Reprocess all repos for a user
  - Authentication via Bearer token in Authorization header
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
- `sysinfo` - System resource monitoring
- `chrono` - Date/time handling
- `serde` - Serialization/deserialization
- `axum` - HTTP framework for health checks and admin API
- `tower` - Middleware framework for authentication