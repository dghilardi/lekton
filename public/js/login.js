// Login form handler for demo mode authentication.
document.getElementById('login-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const username = document.getElementById('login-username').value;
    const password = document.getElementById('login-password').value;
    const errorEl = document.getElementById('login-error');
    const errorMessageEl = document.getElementById('login-error-message');

    errorEl.classList.add('hidden');
    errorMessageEl.textContent = 'Unable to sign in';

    try {
        const res = await fetch('/api/auth/login', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ username, password })
        });
        if (res.ok) {
            window.location.href = '/';
        } else {
            let message = 'Unable to sign in';
            try {
                const data = await res.json();
                if (typeof data.error === 'string' && data.error.trim()) {
                    message = data.error;
                } else if (typeof data.message === 'string' && data.message.trim()) {
                    message = data.message;
                }
            } catch (_) {
                const text = await res.text();
                if (text.trim()) {
                    message = text.trim();
                }
            }
            errorMessageEl.textContent = message;
            errorEl.classList.remove('hidden');
        }
    } catch (err) {
        errorMessageEl.textContent = 'Network error while signing in';
        errorEl.classList.remove('hidden');
    }
});
