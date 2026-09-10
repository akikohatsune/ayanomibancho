function copyText(id, btn) {
            const el = document.getElementById(id);
            if (!el) return;
            const text = el.innerText;
            navigator.clipboard.writeText(text).then(() => {
                const orig = btn.innerText;
                btn.innerText = "Copied!";
                btn.style.color = "#10b981";
                setTimeout(() => {
                    btn.innerText = orig;
                    btn.style.color = "";
                }, 2000);
            }).catch(() => {
                const input = document.createElement('textarea');
                input.value = text;
                document.body.appendChild(input);
                input.select();
                document.execCommand('copy');
                document.body.removeChild(input);
                btn.innerText = "Copied!";
                setTimeout(() => { btn.innerText = "Copy"; }, 2000);
            });
        }
