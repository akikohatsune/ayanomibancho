async function handleAward(e) {
    e.preventDefault();
    const msgEl = document.getElementById('awardMsg');
    const userId = parseInt(document.getElementById('awardUserId').value);
    const badgeId = parseInt(document.getElementById('awardBadgeId').value);
    msgEl.textContent = "Processing...";
    msgEl.style.color = "#94a3b8";

    try {
        const res = await fetch('/api/badges/award', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ user_id: userId, badge_id: badgeId })
        });
        const data = await res.json();
        if (res.ok) {
            msgEl.textContent = "[Success] Badge awarded successfully! Refresh to see changes.";
            msgEl.style.color = "#10b981";
        } else {
            msgEl.textContent = "[Error] " + (data.error || "Failed to award badge");
            msgEl.style.color = "#ef4444";
        }
    } catch(err) {
        msgEl.textContent = "[Error] Server connection error";
        msgEl.style.color = "#ef4444";
    }
}

async function handleCreateBadge(e) {
    e.preventDefault();
    const msgEl = document.getElementById('createBadgeMsg');
    const icon = document.getElementById('newBadgeIcon').value;
    const name = document.getElementById('newBadgeName').value;
    const description = document.getElementById('newBadgeDesc').value;
    msgEl.textContent = "Creating...";
    msgEl.style.color = "#94a3b8";

    try {
        const res = await fetch('/api/badges/create', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ icon, name, description, tag: icon })
        });
        const data = await res.json();
        if (res.ok) {
            msgEl.textContent = "[Success] Badge created successfully! Refresh to update list.";
            msgEl.style.color = "#10b981";
        } else {
            msgEl.textContent = "[Error] " + (data.error || "Failed to create badge");
            msgEl.style.color = "#ef4444";
        }
    } catch(err) {
        msgEl.textContent = "[Error] Server connection error";
        msgEl.style.color = "#ef4444";
    }
}
