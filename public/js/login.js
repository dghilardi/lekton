// Login form handler for demo mode authentication.
document.getElementById('login-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const username = document.getElementById('login-username').value;
    const password = document.getElementById('login-password').value;
    const errorEl = document.getElementById('login-error');

    try {
        const res = await fetch('/api/auth/login', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ username, password })
        });
        if (res.ok) {
            window.location.href = '/';
        } else {
            errorEl.classList.remove('hidden');
        }
    } catch (err) {
        errorEl.classList.remove('hidden');
    }
});
