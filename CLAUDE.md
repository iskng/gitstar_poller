# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

- `cargo run -- --help` - Show CLI help and available commands
- `cargo run -- user <username>` - Analyze a GitHub user's starred repositories
- `cargo run -- repos <url1,url2>` - Analyze specific repositories
- `cargo run -- server` - Run as a background server processing GitHub stars
- `cargo build` - Build the application
- `cargo build --release` - Build optimized release version
- `cargo test` - Run tests
- `cargo check` - Check code without building
- `cargo clippy` - Run Rust linter
- `cargo fmt` - Format code

## Architecture Overview

This is a hybrid CLI tool and server that analyzes GitHub repository stargazers. It can run in two modes:
1. **CLI Mode**: One-time analysis of repositories or users
2. **Server Mode**: Background service that processes GitHub stars using actor-based concurrency

### Project Structure
- `src/main.rs` - Entry point and main application logic
- `src/cli.rs` - Command line argument parsing with clap
- `src/github.rs` - GitHub API client with rate limiting and pagination
- `src/types.rs` - Data structures and type definitions (for CLI mode)
- `src/models.rs` - Data models for server mode and SurrealDB integration
- `src/surreal_client.rs` - SurrealDB client with live query support
- `src/actors/` - Actor system components
  - `token_actor.rs` - Processes repos for a single GitHub token
  - `token_supervisor.rs` - Manages token actors and live queries
- `src/display.rs` - Result formatting and display logic
- `src/json_output.rs` - JSON export functionality
- `src/interactive.rs` - Interactive menu system
- `src/error.rs` - Error handling and custom error types

### Key Features
- **GitHub API Integration**: Fetches stargazers and starred repositories with automatic rate limiting
- **User Analysis**: Finds users who have starred similar repositories
- **Multiple Sorting Options**: Stars, username, repository count, common interests
- **JSON Export**: Save analysis results to JSON files
- **Interactive Mode**: Menu-driven interface for sorting options
- **Rate Limit Handling**: Automatic retries and delays for API limits

### Environment Setup

1. Create a `.env` file from `.env.example`
2. Configure based on your usage:

#### For CLI Mode:
- `GITHUB_TOKEN`: Your GitHub personal access token (required)
- No special scopes required for public repository access

#### For Server Mode:
- `DB_URL`: SurrealDB WebSocket URL (default: ws://localhost:8000)
- `DB_USER`: Database username (default: root)
- `DB_PASS`: Database password (default: root)
- `DB_NAMESPACE`: Must be "gitstars" to match Next.js app
- `DB_DATABASE`: Must be "stars" to match Next.js app

The application automatically loads `.env` on startup. You can also override any setting via command-line arguments.

### Usage Examples
```bash
# Analyze a user's starred repositories
cargo run -- user octocat --limit 50

# Analyze specific repositories
cargo run -- repos "https://github.com/rust-lang/rust,https://github.com/microsoft/vscode"

# Sort by common stars and export to JSON
cargo run -- user octocat --sort common-stars --output results.json

# Interactive mode
cargo run -- user octocat --interactive

# Run as a server (processes GitHub stars in background)
cargo run -- server --db-url ws://localhost:8000 --db-user root --db-pass root
```

### CLI Commands
- `user <username>` - Analyze user's starred repositories
  - `--limit <n>` - Limit number of repositories to analyze
- `repos <urls>` - Analyze comma-separated repository URLs
- `server` - Run as a background server
  - `--db-url <url>` - SurrealDB connection URL
  - `--db-user <user>` - SurrealDB username
  - `--db-pass <pass>` - SurrealDB password
  - `--db-namespace <ns>` - SurrealDB namespace
  - `--db-database <db>` - SurrealDB database
- Global options:
  - `--sort <option>` - Sort results (stars-desc, stars-asc, username, user-repos, common-stars)
  - `--output <file>` - Export results to JSON file
  - `--max-stars <n>` - Skip repositories with more than N stars (default: 50000)
  - `--no-filter` - Include all repositories regardless of star count
  - `--interactive` - Show interactive sorting menu

### Server Mode Architecture

When running in server mode, the application:
1. Connects to SurrealDB and listens for new GitHub accounts via live queries
2. Spawns a TokenActor for each user with a GitHub access token
3. Each TokenActor:
   - Manages its own GitHub API client and rate limits
   - Processes repos the user has starred (already synced by Next.js)
   - Fetches all stargazers for those repos
   - Creates github_user records and stargazer relationships
   - Handles rate limiting with automatic backoff and retry
4. TokenSupervisor manages all TokenActors and provides statistics

The server integrates seamlessly with the Next.js frontend that handles initial user authentication and starred repo synchronization.