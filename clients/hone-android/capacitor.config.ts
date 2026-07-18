import type { CapacitorConfig } from '@capacitor/cli';

const config: CapacitorConfig = {
  appId: 'net.hone.app',
  appName: 'HONE',
  webDir: 'www',
  server: {
    url: 'https://hone.net/app',
    // cleartext: true is required so the webview can reach local LAN nodes
    // over HTTP (192.168.68.72:4242, 192.168.68.75:4242) without being
    // blocked by Android's cleartext traffic policy.
    cleartext: true,
    allowNavigation: ['hone.net', '*.hone.net', '192.168.68.72', '192.168.68.75'],
  },
  plugins: {
    StatusBar: {
      backgroundColor: '#0a0e17',
      style: 'DARK',
    },
  },
  android: {
    buildOptions: {
      keystorePath: undefined,
      keystoreAlias: undefined,
    },
  },
};

export default config;
