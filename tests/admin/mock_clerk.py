#!/usr/bin/env python3
"""
================================================================================
Mock Clerk JWKS & JWT Signing Server
================================================================================

Purpose:
  Provides a self-contained mock implementation of the Clerk auth server. 
  Used by Bridge's admin interface test suite (tests/admin/) to test authentication 
  and token validation endpoints (/admin/*) without relying on external Clerk APIs.

Key Capabilities:
  - Generates RSA keys dynamically on startup.
  - Serves dynamic JWKS keys on GET /.well-known/jwks.json.
  - Issues signed RS256 JWT tokens on GET /token with query parameters to customize 
    claims (expiry, issuer, azp, org_id, or signed with an invalid/wrong key).
  - Implements a self-shutdown hook on GET /shutdown for clean exit.

How to Run:
  Simply run the script with python:
    python mock_clerk.py [port]
  Defaults to port 5577.

How to configure Bridge:
  To make the Bridge server authenticate tokens using this mock server, 
  start the Bridge server with the following environment variables:
    ADMIN_CLERK_FRONTEND_API=http://localhost:5577 \
    ADMIN_CLERK_AUTHORIZED_PARTIES=https://admin.bridge.example.com \
    ADMIN_CLERK_ORG_ID=org_test \
    cargo run
================================================================================
"""
import http.server
import json
import time
import sys
import urllib.parse
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import padding
import base64

# Generate primary RSA key for signing valid tokens
private_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
public_key = private_key.public_key()

# Generate a secondary RSA key to test signature verification failures (wrong key)
wrong_private_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)

def int_to_base64url(val):
    b = val.to_bytes((val.bit_length() + 7) // 8, byteorder='big')
    return base64.urlsafe_b64encode(b).decode('utf-8').rstrip('=')

# Extract components for JWKS
pn = public_key.public_numbers()
n_b64 = int_to_base64url(pn.n)
e_b64 = int_to_base64url(pn.e)

# Generate a dynamic kid to bypass JWKS caching in the Bridge server on rerun
KID = f"mock_key_{int(time.time())}"

jwks = {
    "keys": [
        {
            "kty": "RSA",
            "use": "sig",
            "kid": KID,
            "alg": "RS256",
            "n": n_b64,
            "e": e_b64
        }
    ]
}

def create_jwt(claims, use_wrong_key=False):
    # Header: {"alg": "RS256", "typ": "JWT", "kid": KID}
    header = {"alg": "RS256", "typ": "JWT", "kid": KID}
    header_json = json.dumps(header).encode('utf-8')
    header_b64 = base64.urlsafe_b64encode(header_json).decode('utf-8').rstrip('=')
    
    claims_json = json.dumps(claims).encode('utf-8')
    claims_b64 = base64.urlsafe_b64encode(claims_json).decode('utf-8').rstrip('=')
    
    signing_input = f"{header_b64}.{claims_b64}".encode('utf-8')
    
    pkey = wrong_private_key if use_wrong_key else private_key
    signature = pkey.sign(
        signing_input,
        padding.PKCS1v15(),
        hashes.SHA256()
    )
    sig_b64 = base64.urlsafe_b64encode(signature).decode('utf-8').rstrip('=')
    
    return f"{header_b64}.{claims_b64}.{sig_b64}"

class MockClerkHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass # Suppress HTTP log messages to keep terminal output clean
        
    def do_GET(self):
        parsed_url = urllib.parse.urlparse(self.path)
        if parsed_url.path == "/.well-known/jwks.json":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(jwks).encode('utf-8'))
        elif parsed_url.path == "/token":
            query = urllib.parse.parse_qs(parsed_url.query)
            
            # Default claims
            now = int(time.time())
            claims = {
                "sub": "user_admin_test_123",
                "iss": "http://localhost:5577",
                "exp": now + 3600,
                "iat": now,
                "azp": "https://admin.bridge.example.com"
            }
            
            use_wrong_key = "wrong_key" in query
            
            if "expired" in query:
                claims["exp"] = now - 600
                claims["iat"] = now - 1200
            if "wrong_iss" in query:
                claims["iss"] = "https://wrong.clerk.accounts.dev"
            if "wrong_azp" in query:
                claims["azp"] = "https://evil.example.com"
            if "missing_azp" in query:
                claims.pop("azp", None)
            if "wrong_org" in query:
                claims["org_id"] = "org_wrong"
            elif "org" in query:
                claims["org_id"] = query["org"][0]
            else:
                # Default to org_test
                claims["org_id"] = "org_test"
                
            token = create_jwt(claims, use_wrong_key=use_wrong_key)
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(token.encode('utf-8'))
        elif parsed_url.path == "/shutdown":
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(b"Shutdown initiated")
            import threading
            threading.Thread(target=self.server.shutdown).start()
        else:
            self.send_response(404)
            self.end_headers()

def main():
    port = 5577
    if len(sys.argv) > 1:
        port = int(sys.argv[1])
    server = http.server.HTTPServer(('localhost', port), MockClerkHandler)
    server.serve_forever()

if __name__ == '__main__':
    main()
