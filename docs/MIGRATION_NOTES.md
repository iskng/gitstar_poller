# Migration Notes

## Changes Made

### 1. Fixed Relationship Structure
- Changed from `github_user -> stargazer -> repo` to `github_user -> starred -> repo`
- The `starred` table is the correct one to use for tracking which GitHub users have starred which repositories

### 2. GitHub User Table
- Created proper schema for `github_user` table with fields:
  - githubId (int)
  - username (string)
  - avatarUrl (string) 
  - profileUrl (string)
  - syncedAt (datetime)

### 3. Code Updates
- Updated `insert_stargazers_batch` to create `starred` relationships instead of `stargazer`
- Added proper check to avoid duplicate starred relationships
- Set source as 'stargazer_analysis' to distinguish from user's own stars

## Manual Cleanup Required

### 1. Delete old stargazer relationships
```sql
DELETE stargazer;
```

### 2. Update user table (to be done by frontend)
The user table needs a field to link to github_user records:
```sql
ALTER TABLE user ADD github_users TYPE array<record<github_user>>;
```

## Current State
- github_user table: ~14K records created
- stargazer relationships: 14,691 (need to be deleted)
- starred relationships: Will be created correctly going forward

## Important Notes
- The frontend will handle fixing the site endpoint that creates starred relationships
- Going forward, all star relationships should be `github_user -> starred -> repo`
- Platform users will be linked to their github_user record(s) via the user table