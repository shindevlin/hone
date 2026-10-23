all: hone-sik

hone-sik: fingerprint.cu
	nvcc -O2 -o hone-sik fingerprint.cu

clean:
	rm -f hone-sik

.PHONY: all clean
