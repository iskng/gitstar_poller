-- GitHub Stars Server Schema

-- Define the github_user table
DEFINE TABLE github_user SCHEMAFULL;
DEFINE FIELD github_id ON TABLE github_user TYPE int;
DEFINE FIELD username ON TABLE github_user TYPE string;
DEFINE FIELD avatar_url ON TABLE github_user TYPE string;
DEFINE FIELD profile_url ON TABLE github_user TYPE string;
DEFINE FIELD synced_at ON TABLE github_user TYPE datetime;

-- Indexes for github_user
DEFINE INDEX githubUsernameIndex ON TABLE github_user COLUMNS username UNIQUE;
DEFINE INDEX githubIdIndex ON TABLE github_user COLUMNS github_id UNIQUE;

-- Define the repo_processing_status table (schemaless for flexibility)
DEFINE TABLE repo_processing_status SCHEMALESS;

-- Index for repo_processing_status
DEFINE INDEX repoProcessingRepoIndex ON TABLE repo_processing_status COLUMNS repo;

-- Note: The following tables should already exist from the main app:
-- - user (platform users)
-- - account (OAuth accounts)
-- - repo (GitHub repositories)
-- - starred (relationship: github_user -> starred -> repo)

-- The user table should have a field added to link to github_user:
-- ALTER TABLE user ADD github_users TYPE array<record<github_user>>;