import { writable } from 'svelte/store';
import { browser } from '$app/environment';

export interface Branding {
  name: string;
  shortName: string;
  tagline: string;
  primaryColor: string;
  secondaryColor: string;
  backgroundColor: string;
  surfaceColor: string;
  textColor: string;
  textMutedColor: string;
  fontFamily: string;
  logoPath: string;
  faviconPath: string;
  docsUrl: string;
  supportUrl: string;
}

const defaults: Branding = {
  name: 'Talos Control System',
  shortName: 'TCS',
  tagline: 'Kubernetes Management Simplified',
  primaryColor: '#150D6A',
  secondaryColor: '#4F8BFF',
  backgroundColor: '#0A0A0A',
  surfaceColor: '#1A1A1A',
  textColor: '#FFFFFF',
  textMutedColor: '#A0A0A0',
  fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
  logoPath: '/branding/logo.svg',
  faviconPath: '/branding/favicon.svg',
  docsUrl: '',
  supportUrl: ''
};

export const branding = writable<Branding>({ ...defaults });

export function applyBranding(b: Branding) {
  if (!browser) return;
  
  const root = document.documentElement;
  root.style.setProperty('--tcs-primary', b.primaryColor);
  root.style.setProperty('--tcs-secondary', b.secondaryColor);
  root.style.setProperty('--tcs-background', b.backgroundColor);
  root.style.setProperty('--tcs-surface', b.surfaceColor);
  root.style.setProperty('--tcs-text', b.textColor);
  root.style.setProperty('--tcs-text-muted', b.textMutedColor);
  root.style.setProperty('--tcs-font', b.fontFamily);
  
  document.title = b.shortName;
  
  if (b.faviconPath) {
    const link = document.querySelector('link[rel="icon"]') as HTMLLinkElement | null;
    if (link) link.href = b.faviconPath;
  }
  
  const themeColor = document.querySelector('meta[name="theme-color"]') as HTMLMetaElement | null;
  if (themeColor) themeColor.content = b.primaryColor;
}

export async function fetchBranding(): Promise<void> {
  try {
    const res = await fetch('/api/branding');
    if (!res.ok) return;
    const data = await res.json();
    
    const b: Branding = {
      name: data.name || defaults.name,
      shortName: data.shortName || data.short_name || defaults.shortName,
      tagline: data.tagline || defaults.tagline,
      primaryColor: data.primaryColor || data.primary_color || defaults.primaryColor,
      secondaryColor: data.secondaryColor || data.secondary_color || defaults.secondaryColor,
      backgroundColor: data.backgroundColor || data.background_color || defaults.backgroundColor,
      surfaceColor: data.surfaceColor || data.surface_color || defaults.surfaceColor,
      textColor: data.textColor || data.text_color || defaults.textColor,
      textMutedColor: data.textMutedColor || data.text_muted_color || defaults.textMutedColor,
      fontFamily: data.fontFamily || data.font_family || defaults.fontFamily,
      logoPath: data.logoPath || data.logo_path || defaults.logoPath,
      faviconPath: data.faviconPath || data.favicon_path || defaults.faviconPath,
      docsUrl: data.docsUrl || data.docs_url || '',
      supportUrl: data.supportUrl || data.support_url || ''
    };
    
    branding.set(b);
    applyBranding(b);
  } catch {
    // Use defaults on failure
  }
}
