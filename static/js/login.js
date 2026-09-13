async function handleLogin(e) {
            e.preventDefault();
            const msgEl = document.getElementById('loginMsg');
            const username = document.getElementById('loginUser').value.trim();
            const password = document.getElementById('loginPass').value;
            
            // Extract Cloudflare Turnstile token if present
            const turnstileWidget = document.querySelector('.cf-turnstile');
            const turnstileInput = document.querySelector('[name="cf-turnstile-response"]');
            const cf_turnstile_response = turnstileInput ? turnstileInput.value : undefined;

            if (turnstileWidget && (!cf_turnstile_response || !cf_turnstile_response.trim())) {
                msgEl.textContent = "[Error] Please complete the Cloudflare Turnstile verification before signing in.";
                msgEl.style.color = "#ef4444";
                return;
            }

            msgEl.textContent = "Signing in...";
            msgEl.style.color = "#94a3b8";

            try {
                const res = await fetch('/api/login', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ username, password, cf_turnstile_response })
                });

                let data;
                try {
                    data = await res.json();
                } catch(_) {
                    const text = await res.text().catch(() => "");
                    data = {
                        success: false,
                        message: text || `Server returned error (${res.status} ${res.statusText})`
                    };
                }

                if (res.ok && data.success) {
                    msgEl.textContent = "[Success] " + data.message;
                    msgEl.style.color = "#10b981";
                    setTimeout(() => {
                        window.location.href = '/login';
                    }, 500);
                } else {
                    msgEl.textContent = "[Error] " + (data.message || data.error || "Login failed.");
                    msgEl.style.color = "#ef4444";
                    // Turnstile token lifecycle: tokens are single-use, reset for retry
                    if (window.turnstile) {
                        try { window.turnstile.reset(); } catch(_) {}
                    }
                }
            } catch(err) {
                msgEl.textContent = "[Error] Could not connect to server. Please check your network connection.";
                msgEl.style.color = "#ef4444";
                if (window.turnstile) {
                    try { window.turnstile.reset(); } catch(_) {}
                }
            }
        }
