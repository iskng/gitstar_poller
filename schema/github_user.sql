-- Define the github_user table
DEFINE TABLE github_user SCHEMAFULL;
DEFINE FIELD github_id ON TABLE github_user TYPE int;
DEFINE FIELD username ON TABLE github_user TYPE string;
DEFINE FIELD avatar_url ON TABLE github_user TYPE string;
DEFINE FIELD profile_url ON TABLE github_user TYPE string;
DEFINE FIELD synced_at ON TABLE github_user TYPE datetime;

-- Index for fast lookups
DEFINE INDEX githubUsernameIndex ON TABLE github_user COLUMNS username UNIQUE;
DEFINE INDEX githubIdIndex ON TABLE github_user COLUMNS github_id UNIQUE;