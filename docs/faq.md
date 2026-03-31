# FAQ & Troubleshooting

## Getting Tokens

### How do I get my first BTCPC?
Claim 1 free BTCPC via the faucet — either through the API (`POST /api/faucet/claim`) or the Telegram bot (`/claim`). You can only claim once per account.

### I need more tokens
Email **shin@btcpc.network** with your BTCPC username. Include what you plan to use them for.

### Can I mine without a GPU?
Mining on CPU works but is very slow and earns minimal rewards. An NVIDIA GPU with 8GB+ VRAM is recommended.

## Common Issues

### 2FA Not Working
- Ensure your Telegram account is linked and 2FA is enabled in your account settings.
- Double-check the 2FA code and try again.

### Cannot Connect to Database
- Verify your database connection string in the `.env` file.
- Ensure your MongoDB server is running and accessible.

### Blockchain Account Not Linking
- Make sure you have a valid Hive or TON account.
- Check for typos or missing information in the linking request.

### Staking/Unstaking Fails
- Ensure you have enough tokens to stake or unstake.
- Check if the staking pool exists and is active.

### Inference Returns an Error
- Check you have at least 0.01 BTCPC balance.
- For Telegram bot inference, you need at least 1 BTCPC staked.
- Verify the relay or Ollama endpoint is reachable.

## Support

- GitHub Issues: [shindevlin/btcpc](https://github.com/shindevlin/btcpc/issues)
- Email: shin@btcpc.network
- Telegram Bot: [@btcpcbot](https://t.me/btcpcbot)
