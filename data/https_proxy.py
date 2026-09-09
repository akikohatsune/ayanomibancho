import http.server
import ssl
import threading
import urllib.request
import urllib.error
import os

TARGET = "http://127.0.0.1:5000"

class NoRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None

opener = urllib.request.build_opener(NoRedirectHandler)

class ProxyHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.proxy_request("GET")

    def do_POST(self):
        self.proxy_request("POST")

    def do_HEAD(self):
        self.proxy_request("HEAD")

    def do_DELETE(self):
        self.proxy_request("DELETE")

    def proxy_request(self, method):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length) if content_length > 0 else None

        host = self.headers.get("Host", "")
        req_path = self.path

        # Map avatar requests to /a/{part}
        clean_path = req_path.split('?')[0].rstrip('/')
        part = clean_path.lstrip('/')
        clean_part = part
        for ext in ('.png', '.jpg', '.jpeg', '.webp'):
            if clean_part.endswith(ext):
                clean_part = clean_part[:-len(ext)]
                break

        is_avatar_subdomain = host.startswith("a.") or host.startswith("avatar.")
        is_numeric_root = clean_part.isdigit() and not part.startswith(('u/', 'd/', 'api/', 'web/'))

        if is_avatar_subdomain or is_numeric_root:
            if clean_part.isdigit():
                query_str = ("?" + req_path.split('?', 1)[1]) if '?' in req_path else ""
                req_path = f"/a/{clean_part}{query_str}"

        print(f">>> [PROXY REQ] {method} {req_path} Host: {host} Content-Type: {self.headers.get('Content-Type')}")
        if body:
            print(f">>> [BODY] {body[:8192]}")

        url = f"{TARGET}{req_path}"
        req = urllib.request.Request(url, data=body, method=method)

        for k, v in self.headers.items():
            if k.lower() not in ["host", "content-length", "connection"]:
                req.add_header(k, v)

        proto = "https" if self.server.server_port == 443 else "http"
        req.add_header("X-Forwarded-For", self.client_address[0])
        req.add_header("X-Forwarded-Proto", proto)
        req.add_header("X-Forwarded-Host", host)

        try:
            resp = opener.open(req, timeout=30)
            resp_body = resp.read()
            print(f"<<< [PROXY OK {resp.status}] {method} {req_path}: {resp_body[:200]}")
            self.send_response(resp.status)
            for k, v in resp.headers.items():
                if k.lower() not in ["transfer-encoding", "connection", "content-length", "set-cookie"]:
                    self.send_header(k, v)
            for sc in resp.headers.get_all("Set-Cookie", []):
                self.send_header("Set-Cookie", sc)
            self.send_header("Content-Length", str(len(resp_body)))
            self.end_headers()
            self.wfile.write(resp_body)
        except urllib.error.HTTPError as e:
            err_body = e.read()
            print(f"<<< [PROXY STATUS {e.code}] {method} {req_path}: {err_body[:300]}")
            self.send_response(e.code)
            for k, v in e.headers.items():
                if k.lower() not in ["transfer-encoding", "connection", "content-length", "set-cookie"]:
                    self.send_header(k, v)
            for sc in e.headers.get_all("Set-Cookie", []):
                self.send_header("Set-Cookie", sc)
            self.send_header("Content-Length", str(len(err_body)))
            self.end_headers()
            self.wfile.write(err_body)
        except Exception as e:
            self.send_response(502)
            self.end_headers()
            self.wfile.write(f"Bad Gateway: {e}".encode())

    def log_message(self, format, *args):
        print(f"[{self.server.server_port}] {self.client_address[0]} - {format % args}")

def run_https(port=443):
    cert_path = "data/certs/server.crt"
    key_path = "data/certs/server.key"
    if not os.path.exists(cert_path) or not os.path.exists(key_path):
        print(f"Error: {cert_path} or {key_path} not found!")
        return

    server = http.server.ThreadingHTTPServer(('0.0.0.0', port), ProxyHandler)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(cert_path, key_path)
    server.socket = ctx.wrap_socket(server.socket, server_side=True)
    print(f"[HTTPS Proxy] Online on https://0.0.0.0:{port} -> {TARGET}")
    server.serve_forever()

def run_http(port=80):
    server = http.server.ThreadingHTTPServer(('0.0.0.0', port), ProxyHandler)
    print(f"[HTTP Proxy] Online on http://0.0.0.0:{port} -> {TARGET}")
    server.serve_forever()

if __name__ == '__main__':
    t_http = threading.Thread(target=run_http, args=(80,), daemon=True)
    t_http.start()
    run_https(443)
