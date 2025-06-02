# Database Relationships

## Core Tables

### user
- Platform user account
- Will have a field linking to github_user (to be added by frontend)

### github_user
- GitHub user/account information
- Fields: githubId, username, avatarUrl, profileUrl, syncedAt
- ID format: `github_user:username` (e.g., `github_user:octocat`)

### repo
- GitHub repository information
- ID format: `repo:owner/name` (e.g., `repo:facebook/react`)

### account
- OAuth account information (managed by Better Auth)
- Links platform user to their GitHub OAuth credentials

## Relationships

### starred
- Type: RELATION
- Pattern: `github_user -> starred -> repo`
- Represents which GitHub users have starred which repositories
- Fields: starredAt, source
- Source can be:
  - 'github' - from user's own starred repos
  - 'stargazer_analysis' - discovered by analyzing repo stargazers

## Important Notes

1. **DO NOT** create `user -> starred -> repo` relationships
2. **Always use** `github_user -> starred -> repo` for star relationships
3. The platform user will be linked to their github_user record(s) via a field on the user table
4. A platform user can potentially have multiple GitHub accounts

## Data Flow

1. User logs in with GitHub OAuth → creates `account` record
2. Frontend syncs user's starred repos → creates `github_user` and `starred` relationships
3. Server analyzes repo stargazers → creates more `github_user` records and `starred` relationships
4. When displaying data, query starred relationships for all github_users linked to the platform user