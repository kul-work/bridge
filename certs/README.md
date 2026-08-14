# Certs & Credentials Directory

This directory contains certificate files and Google Play Service Account keys used by Bridge.

## File Inventory

### 1. `ci-mock-google-sa.json`
* **Type:** Non-operational CI Test Mock Fixture.
* **Purpose:** Dummy Google Service Account file used exclusively for offline CI / local shell integration tests (`tests/ci-seed.sql`).
* **Security Status:** **SAFE / MOCK FIXTURE.**
* **Note:** This key/account **does NOT exist in Google Cloud IAM** and **NEVER requires credential rotation**.

### 2. `isrgrootx1.pem`
* **Type:** Public CA Root Certificate (ISRG Root X1 / Let's Encrypt).
* **Purpose:** Trust anchor for outgoing HTTPS/TLS client validation.
* **Security Status:** **PUBLIC CERTIFICATE (Not a secret).**

### 3. `prod-ca-2021.crt`
* **Type:** Public CA Certificate (Supabase Root 2021 CA).
* **Purpose:** SSL certificate authority bundle used for secure PostgreSQL database connections.
* **Security Status:** **PUBLIC CERTIFICATE (Not a secret).**

### 4. `play-billing-482404-49bb8f90e332.json`
* **Type:** Live Google Service Account Private Key (GCP Project: `play-billing-482404`).
* **Purpose:** Authenticates Bridge to Google Play Developer API in production.
* **Security Status:** **SECRET / PRIVILEGED KEY.**
* **Git Status:** Git-ignored (`certs/play-billing*.json` in `.gitignore`). Never commit to Git.

### 5. `play-billing-482519-28c007356bc6.json`
* **Type:** Live Google Service Account Private Key (GCP Project: `play-billing-482519`).
* **Purpose:** Authenticates Bridge to Google Play Developer API in staging/dev.
* **Security Status:** **SECRET / PRIVILEGED KEY.**
* **Git Status:** Git-ignored (`certs/play-billing*.json` in `.gitignore`). Never commit to Git.
