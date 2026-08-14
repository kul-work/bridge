# Certs & Credentials Directory

This directory contains certificate files and Google Play Service Account keys used by Bridge.

## File Naming Convention for Google Play Keys
File format: `play-billing-[GCP_PROJECT_NUM]-[KEY_ID_FINGERPRINT].json`

## File Inventory

### 1. `ci-mock-google-sa.json`
* **App Context:** Shared CI Test Fixture (Offline / Mock).
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
* **App Context:** **HiHa** (Legacy payment client application).
* **GCP Project:** `play-billing-482404` (Project Number: `228709635098`).
* **Service Account:** `service-account@play-billing-482404.iam.gserviceaccount.com`.
* **Key ID:** `49bb8f90e332fd5ff001b5491ebac3bdae8a17fd`.
* **Purpose:** Authenticates Bridge to Google Play Developer API for **HiHa**.
* **Security Status:** **SECRET / PRIVILEGED KEY.**
* **Git Status:** Git-ignored (`certs/play-billing*.json` in `.gitignore`). Never commit to Git.

### 5. `play-billing-482519-28c007356bc6.json`
* **App Context:** **HouseHold** (Current household supplies tracker app).
* **GCP Project:** `play-billing-482519` (Project Number: `985499984160`).
* **Service Account:** `service-account@play-billing-482519.iam.gserviceaccount.com`.
* **Key ID:** `28c007356bc6ae1443780e7b0b6ee2dfb857bffa`.
* **Purpose:** Authenticates Bridge to Google Play Developer API for **HouseHold** (`docs/DB_ONBOARDING.md`).
* **Security Status:** **SECRET / PRIVILEGED KEY.**
* **Git Status:** Git-ignored (`certs/play-billing*.json` in `.gitignore`). Never commit to Git.
