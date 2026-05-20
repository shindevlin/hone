import type { CapacitorConfig } from '@capacitor/cli';

const config: CapacitorConfig = {
  appId: 'net.btcpc.app',
  appName: 'BTCPC',
  webDir: 'www',
  server: {
    url: 'https://btcpc.net/app',
    cleartext: false,
    allowNavigation: ['btcpc.net', '*.btcpc.net'],
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
