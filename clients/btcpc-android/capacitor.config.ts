import type { CapacitorConfig } from '@capacitor/cli';

const config: CapacitorConfig = {
  appId: 'net.btcpc.app',
  appName: 'BTCPC',
  webDir: 'www',
  server: {
    url: 'https://btcpc.net/app',
    // cleartext: true is required so the webview can reach local LAN nodes
    // over HTTP (192.168.68.72:4242, 192.168.68.75:4242) without being
    // blocked by Android's cleartext traffic policy.
    cleartext: true,
    allowNavigation: ['btcpc.net', '*.btcpc.net', '192.168.68.72', '192.168.68.75'],
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
