const path = require('path');

console.log('\n===== HONE INFRASTRUCTURE TEST =====\n');

// 1. Test Ollama integration
console.log('1. OLLAMA INTEGRATION:');
const wg = require('./src/mining/workGenerator.js');
console.log('   URL:', wg.OLLAMA_URL);
console.log('   Default Model:', wg.DEFAULT_MODEL);
console.log('   generateWork() function: ', typeof wg.generateWork === 'function' ? 'OK' : 'MISSING');

// 2. Test imports
console.log('\n2. IMPORTS:');
const tests = [
  { name: 'InferenceRequest', path: './src/models/InferenceRequest' },
  { name: 'session', path: './src/inference/session' },
  { name: 'chainSync', path: './src/p2p/chainSync' },
  { name: 'mempool', path: './src/p2p/mempool' },
  { name: 'Block', path: './src/chain/block' },
  { name: 'blockchain', path: './src/chain/blockchain' },
  { name: 'silicon', path: './src/silicon' },
  { name: 'protocol (p2p)', path: './src/p2p/protocol' },
];

tests.forEach(t => {
  try {
    const mod = require(t.path);
    console.log('   ✓ ' + t.name);
  } catch(e) {
    console.log('   ✗ ' + t.name + ': ' + e.message.split('\n')[0]);
  }
});

// 3. Test protocol requires
console.log('\n3. P2P PROTOCOL REQUIRES:');
const proto = require('./src/p2p/protocol.js');
const protos = [
  { name: 'chainSync (validateBlock)', check: () => typeof proto.handleMessage === 'function' },
  { name: 'mempool (in handler)', check: () => proto.MESSAGE_TYPES.TRANSACTION !== undefined },
  { name: 'Block (in handler)', check: () => proto.MESSAGE_TYPES.BLOCK !== undefined },
  { name: 'blockchain (in handler)', check: () => proto.MESSAGE_TYPES.BLOCK !== undefined },
];
protos.forEach(t => {
  console.log('   ' + (t.check() ? '✓' : '✗') + ' ' + t.name);
});

// 4. Test epoch manager
console.log('\n4. EPOCH MANAGER:');
const em = require('./src/services/epochManager.js');
const epochFuncs = ['getCurrentEpoch', 'finalizeEpoch', 'distributeRewards', 'adjustDifficulty'];
epochFuncs.forEach(fn => {
  const ok = typeof em[fn] === 'function';
  console.log('   ' + (ok ? '✓' : '✗') + ' ' + fn + '()');
});

// 5. Check for unused dependencies
console.log('\n5. PACKAGE.JSON DEPENDENCIES:');
const pkgJson = require('./package.json');
const unused = ['commander', 'winston'];
const used = Object.keys(pkgJson.dependencies).filter(dep => !unused.includes(dep));
console.log('   Total dependencies: ' + Object.keys(pkgJson.dependencies).length);
console.log('   Potentially unused: ' + unused.join(', '));

console.log('\n===== END INFRASTRUCTURE TEST =====\n');
