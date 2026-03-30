all: btcpc-sik

btcpc-sik: fingerprint.cu
	nvcc -O2 -o btcpc-sik fingerprint.cu

clean:
	rm -f btcpc-sik

.PHONY: all clean
