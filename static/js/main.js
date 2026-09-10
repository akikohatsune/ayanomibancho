// AyanomiBancho Shared Frontend Scripts
async function handleLogout(e) {
    if (e) e.preventDefault();
    try {
        await fetch('/api/logout', { method: 'POST' });
    } catch (_) {}
    document.cookie = "ayanomi_session=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0";
    window.location.href = "/login";
}
