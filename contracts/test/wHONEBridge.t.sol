// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";
import "../wHONEBridge.sol";

// ── Minimal ERC-20 mock ───────────────────────────────────────────────────────

contract MockToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function totalSupply() external pure returns (uint256) { return 0; }
}

// ── Test suite ────────────────────────────────────────────────────────────────

contract BridgeTest is Test {

    wHONEBridge bridge;
    MockToken    token;

    // Five deterministic signer private keys.
    uint256 constant PK0 = 0xA11CE;
    uint256 constant PK1 = 0xB0B;
    uint256 constant PK2 = 0xCAFE;
    uint256 constant PK3 = 0xDEAD;
    uint256 constant PK4 = 0xBEEF;

    address s0; address s1; address s2; address s3; address s4;
    address user = address(0x1234);

    uint256 constant POOL_SEED  = 10_000e10;
    uint256 constant DAILY_CAP  = 1_000e10;
    uint256 constant RELEASE_AMT = 100e10;
    uint256 constant RELEASE_NONCE = 1;

    // EIP-712 type hashes (must match the contract).
    bytes32 constant RELEASE_TH  = keccak256("Release(address recipient,uint256 amount,uint256 nonce)");
    bytes32 constant BIND_TH     = keccak256("Bind(string honeUsername,address evmAddress)");
    bytes32 constant ROTATE_TH   = keccak256("RotateSigner(address oldSigner,address newSigner,uint256 nonce)");
    bytes32 constant SETLIM_TH   = keccak256("SetLimits(uint256 dailyReleaseLimit,uint256 dailyLockLimit,uint256 nonce)");

    function setUp() public {
        s0 = vm.addr(PK0); s1 = vm.addr(PK1); s2 = vm.addr(PK2);
        s3 = vm.addr(PK3); s4 = vm.addr(PK4);

        token = new MockToken();
        address[5] memory signers = [s0, s1, s2, s3, s4];
        bridge = new wHONEBridge(address(token), signers, DAILY_CAP, DAILY_CAP);

        // Seed pool
        token.mint(address(bridge), POOL_SEED);

        // Bind "alice" → user via signer s0
        bytes32 d = _bindDigest("alice", user);
        bytes memory sig = _sig(PK0, d);
        bridge.bindAccount("alice", user, sig);
    }

    // ── Happy path: 3-of-5 release ───────────────────────────────────────────

    function testReleaseHappyPath() public {
        uint256 before = token.balanceOf(user);

        bytes[] memory sigs = _releaseSigs(user, RELEASE_AMT, RELEASE_NONCE, PK0, PK1, PK2);
        vm.prank(user);
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, sigs);

        assertEq(token.balanceOf(user), before + RELEASE_AMT, "user must receive tokens");
        assertEq(token.balanceOf(address(bridge)), POOL_SEED - RELEASE_AMT, "pool decremented");
    }

    // ── 2 sigs insufficient ──────────────────────────────────────────────────

    function testReleaseFails2Sigs() public {
        bytes[] memory sigs = new bytes[](2);
        bytes32 d = _releaseDigest(user, RELEASE_AMT, RELEASE_NONCE);
        sigs[0] = _sig(PK0, d);
        sigs[1] = _sig(PK1, d);

        vm.prank(user);
        vm.expectRevert("Insufficient valid signatures");
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, sigs);
    }

    // ── Nonce replay ─────────────────────────────────────────────────────────

    function testReleaseReplayRejected() public {
        bytes[] memory sigs = _releaseSigs(user, RELEASE_AMT, RELEASE_NONCE, PK0, PK1, PK2);

        vm.startPrank(user);
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, sigs);

        // Re-sign with fresh sigs but same nonce.
        bytes[] memory sigs2 = _releaseSigs(user, RELEASE_AMT, RELEASE_NONCE, PK0, PK1, PK2);
        vm.expectRevert("Nonce already used");
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, sigs2);
        vm.stopPrank();
    }

    // ── Duplicate signer counted once ────────────────────────────────────────

    function testReleaseDuplicateSignerNotCounted() public {
        bytes32 d = _releaseDigest(user, RELEASE_AMT, RELEASE_NONCE);
        bytes[] memory sigs = new bytes[](3);
        sigs[0] = _sig(PK0, d); // signer 0
        sigs[1] = _sig(PK0, d); // duplicate — should not count
        sigs[2] = _sig(PK1, d); // signer 1 — only 2 unique → fail

        vm.prank(user);
        vm.expectRevert("Insufficient valid signatures");
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, sigs);
    }

    // ── Non-signer signature ignored ─────────────────────────────────────────

    function testReleaseNonSignerIgnored() public {
        bytes32 d = _releaseDigest(user, RELEASE_AMT, RELEASE_NONCE);
        bytes[] memory sigs = new bytes[](3);
        sigs[0] = _sig(PK0, d);           // valid signer
        sigs[1] = _sig(PK1, d);           // valid signer
        sigs[2] = _sig(0xFACE, d);        // not a signer — ignored

        vm.prank(user);
        vm.expectRevert("Insufficient valid signatures");
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, sigs);
    }

    // ── Daily release cap ────────────────────────────────────────────────────

    function testDailyReleaseLimitEnforced() public {
        // First release: fills the daily cap.
        bytes[] memory sigs = _releaseSigs(user, DAILY_CAP, 1, PK0, PK1, PK2);
        vm.prank(user);
        bridge.releaseFromBridge(DAILY_CAP, 1, sigs);

        // Second release: exceeds cap.
        bytes[] memory sigs2 = _releaseSigs(user, 1, 2, PK0, PK1, PK2);
        vm.prank(user);
        vm.expectRevert("Daily release limit reached");
        bridge.releaseFromBridge(1, 2, sigs2);
    }

    // ── Daily lock cap ───────────────────────────────────────────────────────

    function testDailyLockLimitEnforced() public {
        // Approve and lock DAILY_CAP worth.
        vm.startPrank(user);
        token.approve(address(bridge), type(uint256).max);

        // Give user enough tokens.
        token.mint(user, DAILY_CAP + 1);

        bridge.lockForBridge(DAILY_CAP, "alice");

        vm.expectRevert("Daily lock limit reached");
        bridge.lockForBridge(1, "alice");
        vm.stopPrank();
    }

    // ── Pause: any signer can pause ──────────────────────────────────────────

    function testPauseBlocksRelease() public {
        bytes memory pauseSig = _sig(PK0, _pauseDigest());
        bridge.pause(pauseSig);
        assertTrue(bridge.paused(), "bridge must be paused");

        bytes[] memory sigs = _releaseSigs(user, RELEASE_AMT, RELEASE_NONCE, PK0, PK1, PK2);
        vm.prank(user);
        vm.expectRevert("Bridge: paused");
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, sigs);
    }

    function testNonSignerCannotPause() public {
        // Sign with a non-signer key.
        bytes memory badSig = _sig(0xBAD, _pauseDigest());
        vm.expectRevert("Not an authorised signer");
        bridge.pause(badSig);
    }

    // ── Unpause requires 3-of-5 ──────────────────────────────────────────────

    function testUnpauseRequires3Sigs() public {
        bridge.pause(_sig(PK0, _pauseDigest()));

        bytes32 ud = _unpauseDigest();

        // Two sigs — not enough.
        bytes[] memory two = new bytes[](2);
        two[0] = _sig(PK0, ud);
        two[1] = _sig(PK1, ud);
        vm.expectRevert("Insufficient valid signatures");
        bridge.unpause(two);

        // Three sigs — succeeds.
        bytes[] memory three = new bytes[](3);
        three[0] = _sig(PK0, ud);
        three[1] = _sig(PK1, ud);
        three[2] = _sig(PK2, ud);
        bridge.unpause(three);
        assertFalse(bridge.paused(), "bridge must be unpaused");
    }

    // ── Signer rotation ──────────────────────────────────────────────────────

    function testSignerRotation() public {
        uint256 pkNew = 6;
        address newSigner = vm.addr(pkNew);
        uint256 rNonce = bridge.rotateNonce(); // 0

        bytes32 d = _rotateDigest(s0, newSigner, rNonce);
        bytes[] memory sigs = new bytes[](3);
        sigs[0] = _sig(PK0, d);
        sigs[1] = _sig(PK1, d);
        sigs[2] = _sig(PK2, d);
        bridge.rotateSigner(s0, newSigner, sigs);

        // s0 is no longer a signer — its sig on a release should be rejected.
        bytes32 relD = _releaseDigest(user, RELEASE_AMT, RELEASE_NONCE);
        bytes[] memory badSigs = new bytes[](3);
        badSigs[0] = _sig(PK0, relD);    // old signer — no longer valid
        badSigs[1] = _sig(PK1, relD);
        badSigs[2] = _sig(PK2, relD);

        vm.prank(user);
        vm.expectRevert("Insufficient valid signatures");
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, badSigs);

        // newSigner's key (0xNEW1) works instead.
        bytes[] memory goodSigs = new bytes[](3);
        goodSigs[0] = _sig(pkNew, relD);
        goodSigs[1] = _sig(PK1, relD);
        goodSigs[2] = _sig(PK2, relD);
        vm.prank(user);
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, goodSigs);
        assertEq(token.balanceOf(user), RELEASE_AMT);
    }

    // ── Bind: non-signer rejected ─────────────────────────────────────────────

    function testBindNonSignerRejected() public {
        address eve = address(0xEEE);
        bytes32 d = _bindDigest("eve", eve);
        bytes memory badSig = _sig(0xBAD, d); // not a signer
        vm.expectRevert("Not an authorised signer");
        bridge.bindAccount("eve", eve, badSig);
    }

    // ── Double-bind rejected ──────────────────────────────────────────────────

    function testBindAlreadyBoundRejected() public {
        // "alice" is already bound in setUp.
        address other = address(0x9999);
        bytes32 d = _bindDigest("alice", other);
        bytes memory sig = _sig(PK1, d);
        vm.expectRevert("Username already bound");
        bridge.bindAccount("alice", other, sig);
    }

    // ── Unbound wallet cannot release ────────────────────────────────────────

    function testUnboundWalletCannotRelease() public {
        address stranger = address(0xABCD);
        bytes[] memory sigs = _releaseSigs(stranger, RELEASE_AMT, RELEASE_NONCE, PK0, PK1, PK2);
        vm.prank(stranger);
        vm.expectRevert("Wallet not bound");
        bridge.releaseFromBridge(RELEASE_AMT, RELEASE_NONCE, sigs);
    }

    // ── Daily limit resets next day ───────────────────────────────────────────

    function testDailyLimitResetsNextDay() public {
        // Pre-compute sigs before vm.prank so the prank isn't consumed by DOMAIN_SEPARATOR().
        bytes[] memory sigs1 = _releaseSigs(user, DAILY_CAP, 1, PK0, PK1, PK2);
        vm.prank(user);
        bridge.releaseFromBridge(DAILY_CAP, 1, sigs1);

        vm.warp(block.timestamp + 86400);

        bytes[] memory sigs2 = _releaseSigs(user, RELEASE_AMT, 2, PK0, PK1, PK2);
        vm.prank(user);
        bridge.releaseFromBridge(RELEASE_AMT, 2, sigs2);
        assertEq(token.balanceOf(user), DAILY_CAP + RELEASE_AMT);
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    function _releaseDigest(address recipient, uint256 amount, uint256 nonce)
        internal view returns (bytes32)
    {
        bytes32 sh = keccak256(abi.encode(RELEASE_TH, recipient, amount, nonce));
        return keccak256(abi.encodePacked("\x19\x01", bridge.DOMAIN_SEPARATOR(), sh));
    }

    function _bindDigest(string memory username, address evmAddr)
        internal view returns (bytes32)
    {
        bytes32 sh = keccak256(abi.encode(BIND_TH, keccak256(bytes(username)), evmAddr));
        return keccak256(abi.encodePacked("\x19\x01", bridge.DOMAIN_SEPARATOR(), sh));
    }

    function _rotateDigest(address oldSigner, address newSigner, uint256 nonce)
        internal view returns (bytes32)
    {
        bytes32 sh = keccak256(abi.encode(ROTATE_TH, oldSigner, newSigner, nonce));
        return keccak256(abi.encodePacked("\x19\x01", bridge.DOMAIN_SEPARATOR(), sh));
    }

    function _sig(uint256 pk, bytes32 digest) internal pure returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        return abi.encodePacked(r, s, v);
    }

    function _releaseSigs(
        address recipient, uint256 amount, uint256 nonce,
        uint256 pk0_, uint256 pk1_, uint256 pk2_
    ) internal view returns (bytes[] memory) {
        bytes32 d = _releaseDigest(recipient, amount, nonce);
        bytes[] memory sigs = new bytes[](3);
        sigs[0] = _sig(pk0_, d);
        sigs[1] = _sig(pk1_, d);
        sigs[2] = _sig(pk2_, d);
        return sigs;
    }

    function _pauseDigest() internal view returns (bytes32) {
        bytes32 sh = keccak256(abi.encode(keccak256("Pause()"), true));
        return keccak256(abi.encodePacked("\x19\x01", bridge.DOMAIN_SEPARATOR(), sh));
    }

    function _unpauseDigest() internal view returns (bytes32) {
        bytes32 sh = keccak256(abi.encode(keccak256("Unpause()"), false));
        return keccak256(abi.encodePacked("\x19\x01", bridge.DOMAIN_SEPARATOR(), sh));
    }
}
