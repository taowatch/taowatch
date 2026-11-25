-- Optional username/password sign-in (Argon2id PHC string). NULL = passkey-only account.
ALTER TABLE users ADD COLUMN password_hash text;
