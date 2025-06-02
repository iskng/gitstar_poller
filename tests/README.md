# GitHub Stars Server Tests

This directory contains integration and unit tests for the GitHub Stars server.

## Test Structure

- `common/mod.rs` - Shared test utilities and database context
- `database_queries.rs` - Integration tests for SurrealDB queries
- `github_client.rs` - Tests for GitHub API client
- `actor_system.rs` - Tests for the actor-based processing system
- `error_handling.rs` - Unit tests for error types
- `models.rs` - Unit tests for data models

## Running Tests

### Prerequisites

1. Ensure SurrealDB is running on localhost:8000 with the dev database
2. Set up environment variables (optional):
   ```bash
   export DB_URL=ws://localhost:8000
   export DB_USER=root
   export DB_PASS=root
   export DB_NAMESPACE=gitstars
   export DB_DATABASE=stars
   ```

3. For GitHub API tests, set a GitHub token:
   ```bash
   export GITHUB_TOKEN=your_github_token
   ```

### Running All Tests

```bash
# Run all tests (skips tests requiring GitHub token)
cargo test

# Run all tests including GitHub API tests
GITHUB_TOKEN=your_token cargo test -- --include-ignored

# Run with verbose output
cargo test -- --nocapture
```

### Running Specific Test Categories

```bash
# Database integration tests only
cargo test database_queries

# Actor system tests only
cargo test actor_system

# GitHub client tests (requires token)
GITHUB_TOKEN=your_token cargo test github_client -- --include-ignored

# Unit tests only
cargo test models error_handling
```

### Test Database Requirements

The integration tests expect a running SurrealDB instance with:
- At least one GitHub account with a valid access token
- At least one user with starred repositories
- The schema from the Next.js application

Tests are designed to work with existing dev data without modifying it destructively.

## Test Coverage

The test suite covers:

1. **Database Operations**:
   - Connection establishment
   - Account queries
   - User information retrieval
   - Repository listing
   - Repo claiming mechanism
   - Stargazer batch saving
   - Live query setup

2. **GitHub API Client**:
   - Client creation
   - Repository info fetching
   - Stargazer pagination
   - Rate limit handling
   - Error scenarios

3. **Actor System**:
   - Token actor spawning
   - Supervisor management
   - Account processing
   - Live query integration
   - Graceful shutdown

4. **Error Handling**:
   - Error type conversions
   - Error display formatting
   - Result type usage

5. **Data Models**:
   - Struct creation
   - Serialization/deserialization
   - State transitions

## Notes

- Integration tests use the actual dev database, so results may vary based on existing data
- GitHub API tests are marked with `#[ignore]` to prevent rate limit issues in CI
- Actor tests use short timeouts to avoid long test runs
- Some tests may print warnings about rate limits - this is expected behavior