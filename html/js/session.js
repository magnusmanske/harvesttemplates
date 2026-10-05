// Who is logged in, shared by all components.
import { ref } from 'vue';
import { api } from './api.js';

export const user = ref(null);

export async function refreshUser() {
  try {
    user.value = (await api('/auth/me')).user;
  } catch {
    user.value = null;
  }
}

export function login() {
  const here = location.pathname + location.search + location.hash;
  location.href = `/api/auth/login?return_to=${encodeURIComponent(here)}`;
}

export async function logout() {
  await api('/auth/logout', { method: 'POST' });
  user.value = null;
}
