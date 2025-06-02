# GitHub Stars Analyzer

A dual-mode Rust application that analyzes GitHub repository stargazers. It can run as a CLI tool for one-time analysis or as a background server that processes GitHub stars in real-time.

## Features

- **CLI Mode**: Analyze specific repositories or a user's starred repos
- **Server Mode**: Background processing with SurrealDB integration
- **Rate Limit Management**: Automatic handling of GitHub API limits
- **Actor-Based Concurrency**: Efficient parallel processing
- **Live Queries**: Real-time detection of new GitHub accounts

## Installation

```bash
# Build the project
cargo build --release

# The binary will be at ./target/release/github-stars
```

## Usage

### CLI Mode

```bash
# Analyze a user's starred repositories
./github-stars user octocat --limit 50

# Analyze specific repositories
./github-stars repos "https://github.com/rust-lang/rust,https://github.com/tokio-rs/tokio"

# Export results to JSON
./github-stars user octocat --sort common-stars --output results.json
```

### Server Mode

```bash
# Run with default settings (connects to SurrealDB at ws://localhost:8000)
./github-stars server

# Run with custom database settings
./github-stars server \
  --db-url ws://localhost:8000 \
  --db-user root \
  --db-pass mypassword \
  --db-namespace gitstars \
  --db-database stars

# Or use environment variables
export DB_URL=ws://localhost:8000
export DB_USER=root
export DB_PASS=mypassword
export DB_NAMESPACE=gitstars
export DB_DATABASE=stars
./github-stars server
```

## Configuration

### SurrealDB Requirements

The server mode requires SurrealDB to be running and accessible. The database schema should match the one used by the Next.js frontend application.

**Required namespace**: `gitstars`  
**Required database**: `stars`

### Environment Variables

Create a `.env` file in the project root (copy from `.env.example`):

```env
# For CLI mode (required when using 'user' or 'repos' commands)
GITHUB_TOKEN=your_github_personal_access_token

# For Server mode (these override the defaults)
DB_URL=ws://localhost:8000
DB_USER=root
DB_PASS=root
DB_NAMESPACE=gitstars
DB_DATABASE=stars
```

The application automatically loads the `.env` file on startup. Command-line arguments take precedence over environment variables.

## How It Works

### Server Mode Architecture

1. **Live Query Monitoring**: Watches for new GitHub accounts added by the Next.js app
2. **Token Actor Creation**: Spawns an actor for each user with a GitHub token
3. **Repository Processing**: Each actor:
   - Fetches repos the user has starred (already synced by Next.js)
   - Claims unprocessed repos atomically in the database
   - Fetches all stargazers for those repos
   - Creates `github_user` records and relationships
4. **Rate Limit Management**: Each actor manages its own rate limits independently

### Database Schema

The application expects these tables in SurrealDB:
- `account`: GitHub OAuth accounts with access tokens
- `user`: Application users
- `repo`: GitHub repositories
- `starred`: User->Repo relationships
- `repo_processing_status`: Tracking processing state
- `github_user`: GitHub users who have starred repos
- `stargazer`: GitHubUser->Repo relationships

## Development

```bash
# Run tests
cargo test

# Check code
cargo check

# Format code
cargo fmt

# Run linter
cargo clippy
```

## Integration with Next.js

This server is designed to work alongside a Next.js application that:
1. Handles GitHub OAuth authentication
2. Syncs users' starred repositories
3. Creates the initial database records

The Rust server then processes these repos in the background to find all stargazers.