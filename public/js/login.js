// Login form handler for demo mode authentication.

// The page to return to, checked again here like `auth::return_to`: only a
// local path, never `//host` or a backslash variant, and never the login page.
function returnTo(path) {
    if (typeof path !== 'string' || !path.startsWith('/') || path.startsWith('//')
        || path.includes('\\') || /[\u0000-\u001f\u007f]/.test(path)) {
        return '/';
    }
    const route = path.split(/[?#]/)[0];
    if (route === '/login' || route.startsWith('/login/') || route.startsWith('/auth/') || route.startsWith('/api/')) {
        return '/';
    }
    return path;
}

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
            window.location.href = returnTo(e.target.dataset.returnTo);
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
