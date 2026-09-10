let currentBio = typeof INITIAL_RAW_BIO !== 'undefined' ? INITIAL_RAW_BIO : "";

        function showToast(msg, type = 'success') {
            let toast = document.getElementById('profileToast');
            if (!toast) {
                toast = document.createElement('div');
                toast.id = 'profileToast';
                toast.className = 'profile-toast';
                document.body.appendChild(toast);
            }
            toast.textContent = msg;
            toast.className = 'profile-toast show ' + (type === 'error' ? 'toast-error' : 'toast-success');
            clearTimeout(window.__toastTimer);
            window.__toastTimer = setTimeout(() => {
                toast.className = 'profile-toast';
            }, 3500);
        }

        function parseBioMarkdown(raw) {
            if (!raw || !raw.trim()) {
                return '<p style="color: var(--text-muted); font-style: italic;">No bio written yet. Click \'Edit\' to share something about yourself!</p>';
            }
            if (window.marked && typeof window.marked.parse === 'function') {
                try {
                    return window.marked.parse(raw, { breaks: true, gfm: true });
                } catch(e) {
                    console.warn('marked parse error:', e);
                }
            }
            // Fallback safe markdown parser
            let s = raw.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
            s = s.replace(/```([\s\S]*?)```/g, '<pre><code>$1</code></pre>');
            s = s.replace(/`([^`]+)`/g, '<code>$1</code>');
            s = s.replace(/^### (.*$)/gim, '<h3>$1</h3>');
            s = s.replace(/^## (.*$)/gim, '<h2>$1</h2>');
            s = s.replace(/^# (.*$)/gim, '<h1>$1</h1>');
            s = s.replace(/^\> (.*$)/gim, '<blockquote>$1</blockquote>');
            s = s.replace(/\*\*\*(.*?)\*\*\*/g, '<b><i>$1</i></b>');
            s = s.replace(/\*\*(.*?)\*\*/g, '<b>$1</b>');
            s = s.replace(/\*(.*?)\*/g, '<i>$1</i>');
            s = s.replace(/~~(.*?)~~/g, '<del>$1</del>');
            s = s.replace(/!\[(.*?)\]\((.*?)\)/g, '<img alt="$1" src="$2" style="max-width: 100%; border-radius: 6px; margin: 0.5rem 0;" />');
            s = s.replace(/\[(.*?)\]\((.*?)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer" style="color: #f472b6; text-decoration: underline;">$1</a>');
            s = s.replace(/^\s*-\s+(.*$)/gim, '<li>$1</li>');
            s = s.replace(/\n\n+/g, '</p><p>');
            s = s.replace(/\n/g, '<br>');
            return '<p>' + s + '</p>';
        }

        document.addEventListener('DOMContentLoaded', () => {
            const bioContentEl = document.getElementById('bioContent');
            if (bioContentEl) {
                bioContentEl.innerHTML = parseBioMarkdown(currentBio);
            }
            const bioEditorInput = document.getElementById('bioEditorInput');
            if (bioEditorInput) {
                bioEditorInput.addEventListener('input', updateBioCounter);
            }
        });

        function toggleBioEdit(show) {
            const viewMode = document.getElementById('bioViewMode');
            const editMode = document.getElementById('bioEditMode');
            const editBtn = document.getElementById('btnBioEdit');
            if (show) {
                viewMode.style.display = 'none';
                editMode.style.display = 'block';
                if (editBtn) editBtn.style.display = 'none';
                const textarea = document.getElementById('bioEditorInput');
                textarea.value = currentBio;
                updateBioCounter();
                setEditorTab('write');
                textarea.focus();
            } else {
                viewMode.style.display = 'block';
                editMode.style.display = 'none';
                if (editBtn) editBtn.style.display = 'flex';
            }
        }

        function setEditorTab(tab) {
            const tabWrite = document.getElementById('tabWrite');
            const tabPreview = document.getElementById('tabPreview');
            const editorWrite = document.getElementById('editorWriteArea');
            const editorPreview = document.getElementById('editorPreviewArea');
            if (tab === 'write') {
                tabWrite.classList.add('active');
                tabPreview.classList.remove('active');
                editorWrite.style.display = 'block';
                editorPreview.style.display = 'none';
            } else {
                tabWrite.classList.remove('active');
                tabPreview.classList.add('active');
                editorWrite.style.display = 'none';
                editorPreview.style.display = 'block';
                const val = document.getElementById('bioEditorInput').value;
                editorPreview.innerHTML = parseBioMarkdown(val);
            }
        }

        function insertMarkdown(prefix, suffix, defaultText = 'text') {
            const el = document.getElementById('bioEditorInput');
            const start = el.selectionStart;
            const end = el.selectionEnd;
            const text = el.value;
            const selected = text.substring(start, end) || defaultText;
            const replacement = prefix + selected + suffix;
            el.value = text.substring(0, start) + replacement + text.substring(end);
            el.focus();
            el.setSelectionRange(start + prefix.length, start + prefix.length + selected.length);
            updateBioCounter();
        }

        function updateBioCounter() {
            const el = document.getElementById('bioEditorInput');
            const cnt = document.getElementById('bioCharCount');
            if (el && cnt) cnt.textContent = el.value.length;
        }

        async function saveBioEdit() {
            const newBio = document.getElementById('bioEditorInput').value;
            const btn = document.getElementById('btnSaveBio');
            const origHtml = btn.innerHTML;
            btn.disabled = true;
            btn.textContent = "Saving...";

            try {
                const res = await fetch('/api/profile/update', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ bio: newBio })
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    currentBio = newBio;
                    document.getElementById('bioContent').innerHTML = parseBioMarkdown(currentBio);
                    toggleBioEdit(false);
                    showToast("Bio updated successfully!", "success");
                } else {
                    showToast(data.message || "Failed to save bio.", "error");
                }
            } catch(e) {
                showToast("Connection error while saving bio.", "error");
            } finally {
                btn.disabled = false;
                btn.innerHTML = origHtml;
            }
        }

        async function handleAvatarUpload(e) {
            const file = e.target.files[0];
            if (!file) return;
            if (file.size > 5 * 1024 * 1024) {
                showToast("Avatar file exceeds 5MB limit.", "error");
                return;
            }
            showToast("Uploading avatar...", "info");
            const fd = new FormData();
            fd.append('avatar', file);

            try {
                const res = await fetch('/api/profile/avatar', {
                    method: 'POST',
                    body: fd
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    const v = Date.now();
                    document.querySelectorAll('img[src*="/a/"]').forEach(img => {
                        img.src = img.src.split('?')[0] + '?v=' + v;
                    });
                    showToast("Avatar updated successfully!", "success");
                } else {
                    showToast(data.message || "Failed to upload avatar.", "error");
                }
            } catch(err) {
                showToast("Connection error uploading avatar.", "error");
            } finally {
                e.target.value = '';
            }
        }

        async function handleAvatarReset(e) {
            if (e) e.preventDefault();
            if (!confirm("Reset avatar to default (Marisa)?")) return;
            showToast("Resetting avatar...", "info");
            try {
                const res = await fetch('/api/profile/avatar/reset', { method: 'POST' });
                const data = await res.json();
                if (res.ok && data.success) {
                    const v = Date.now();
                    document.querySelectorAll('img[src*="/a/"]').forEach(img => {
                        img.src = img.src.split('?')[0] + '?v=' + v;
                    });
                    showToast("Avatar reset to default!", "success");
                } else {
                    showToast(data.message || "Failed to reset avatar.", "error");
                }
            } catch(err) {
                showToast("Connection error resetting avatar.", "error");
            }
        }

        async function handleBannerUpload(e) {
            const file = e.target.files[0];
            if (!file) return;
            if (file.size > 10 * 1024 * 1024) {
                showToast("Banner image exceeds 10MB limit.", "error");
                return;
            }
            showToast("Uploading banner...", "info");
            const fd = new FormData();
            fd.append('banner', file);

            try {
                const res = await fetch('/api/profile/banner', {
                    method: 'POST',
                    body: fd
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    const v = Date.now();
                    const cover = document.getElementById('profileCover');
                    if (cover) {
                        const uid = cover.dataset.userId;
                        cover.style.backgroundImage = `linear-gradient(180deg, rgba(15, 23, 42, 0.25) 0%, rgba(15, 23, 42, 0.75) 55%, rgba(15, 23, 42, 0.96) 100%), url('/b/${uid}?v=${v}')`;
                    }
                    showToast("Banner updated successfully!", "success");
                } else {
                    showToast(data.message || "Failed to upload banner.", "error");
                }
            } catch(err) {
                showToast("Connection error uploading banner.", "error");
            } finally {
                e.target.value = '';
            }
        }

        async function handleBannerReset(e) {
            if (e) e.preventDefault();
            if (!confirm("Reset profile cover banner to default?")) return;
            showToast("Resetting banner...", "info");
            try {
                const res = await fetch('/api/profile/banner/reset', { method: 'POST' });
                const data = await res.json();
                if (res.ok && data.success) {
                    const v = Date.now();
                    const cover = document.getElementById('profileCover');
                    if (cover) {
                        const uid = cover.dataset.userId;
                        cover.style.backgroundImage = `linear-gradient(180deg, rgba(15, 23, 42, 0.25) 0%, rgba(15, 23, 42, 0.75) 55%, rgba(15, 23, 42, 0.96) 100%), url('/b/${uid}?v=${v}')`;
                    }
                    showToast("Banner reset to default!", "success");
                } else {
                    showToast(data.message || "Failed to reset banner.", "error");
                }
            } catch(err) {
                showToast("Connection error resetting banner.", "error");
            }
        }

        function openCountryModal() {
            const modal = document.getElementById('countryModal');
            if (modal) modal.style.display = 'flex';
        }

        function closeCountryModal() {
            const modal = document.getElementById('countryModal');
            if (modal) modal.style.display = 'none';
        }

        async function saveCountryChange() {
            const select = document.getElementById('countryModalSelect');
            if (!select) return;
            const cid = select.value;
            const opt = select.options[select.selectedIndex];
            const flag = opt ? opt.getAttribute('data-flag') || '' : '';
            const code = opt ? opt.getAttribute('data-code') || '' : '';
            const name = opt ? opt.getAttribute('data-name') || '' : '';

            closeCountryModal();
            showToast("Updating country...", "info");

            try {
                const res = await fetch('/api/profile/update', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ country: cid })
                });
                const data = await res.json();
                if (res.ok && data.success) {
                    const el = document.getElementById('countryDisplayTxt');
                    if (el) el.textContent = `${flag} ${name} (${code})`;
                    showToast("Country updated successfully!", "success");
                } else {
                    showToast(data.message || "Failed to update country.", "error");
                }
            } catch(err) {
                showToast("Connection error updating country.", "error");
            }
        }
