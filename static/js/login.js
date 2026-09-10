async function handleLogin(e) {
            e.preventDefault();
            const msgEl = document.getElementById('loginMsg');
            const username = document.getElementById('loginUser').value.trim();
            const password = document.getElementById('loginPass').value;
            
            // Extract Cloudflare Turnstile token if present
            const turnstileInput = document.querySelector('[name="cf-turnstile-response"]');
            const cf_turnstile_response = turnstileInput ? turnstileInput.value : undefined;

            msgEl.textContent = "Signing in...";
            msgEl.style.color = "#94a3b8";

            try {
                const res = await fetch('/api/login', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ username, password, cf_turnstile_response })
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    msgEl.textContent = "[Success] " + data.message;
                    msgEl.style.color = "#10b981";
                    setTimeout(() => {
                        window.location.href = '/login';
                    }, 500);
                } else {
                    msgEl.textContent = "[Error] " + (data.message || "Login failed.");
                    msgEl.style.color = "#ef4444";
                    // Turnstile token lifecycle: tokens are single-use, reset for retry
                    if (window.turnstile) {
                        try { window.turnstile.reset(); } catch(_) {}
                    }
                }
            } catch(err) {
                msgEl.textContent = "[Error] Server connection error.";
                msgEl.style.color = "#ef4444";
                if (window.turnstile) {
                    try { window.turnstile.reset(); } catch(_) {}
                }
            }
        }
