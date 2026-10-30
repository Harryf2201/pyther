# Final executable assembly evidence

Extracts from the actual native release executables, using installed SDK dumpbin.
Library-only output differs after fat LTO and is not the final default code.
The old dual executable was saved before edits. Addresses are local PE addresses.


## xsearch5

```asm
  000000014001E4D2: C4 E2 7D 18 F6     vbroadcastss ymm6,xmm6
  000000014001E4D7: C5 CC 59 78 A0     vmulps      ymm7,ymm6,ymmword ptr [rax-60h]
  000000014001E4DC: C5 FC 58 C7        vaddps      ymm0,ymm0,ymm7
  000000014001E4E0: C5 CC 59 78 C0     vmulps      ymm7,ymm6,ymmword ptr [rax-40h]
  000000014001E4E5: C5 F4 58 CF        vaddps      ymm1,ymm1,ymm7
  000000014001E4E9: C5 CC 59 78 E0     vmulps      ymm7,ymm6,ymmword ptr [rax-20h]
  000000014001E4EE: C5 EC 58 D7        vaddps      ymm2,ymm2,ymm7
  000000014001E4F2: C5 CC 59 30        vmulps      ymm6,ymm6,ymmword ptr [rax]
  000000014001E4F6: C5 E4 58 DE        vaddps      ymm3,ymm3,ymm6
  000000014001E4FA: 48 05 00 04 00 00  add         rax,400h
  000000014001E500: 48 83 F9 40        cmp         rcx,40h
  000000014001E504: 75 BA              jne         000000014001E4C0
  000000014001E506: C5 FC 29 44 24 60  vmovaps     ymmword ptr [rsp+60h],ymm0
  000000014001E50C: C5 FC 29 8C 24 80  vmovaps     ymmword ptr [rsp+0000000000000080h],ymm1
  000000014001E515: C5 FC 29 94 24 A0  vmovaps     ymmword ptr [rsp+00000000000000A0h],ymm2
  000000014001E51E: C5 FC 29 9C 24 C0  vmovaps     ymmword ptr [rsp+00000000000000C0h],ymm3
  000000014001E527: C4 A1 7A 10 8C B3  vmovss      xmm1,dword ptr [rbx+r14*4+0000000000205200h]
  000000014001E531: 4A 8D 04 B3        lea         rax,[rbx+r14*4]
  000000014001E535: 48 05 60 4E 20 00  add         rax,204E60h
  000000014001E53B: 31 C9              xor         ecx,ecx
  000000014001E53D: C5 F8 57 C0        vxorps      xmm0,xmm0,xmm0
  000000014001E541: C4 E2 79 18 15 56  vbroadcastss xmm2,dword ptr [000000014006D6A0h]
  000000014001E54A: 66 0F 1F 44 00 00  nop         word ptr [rax+rax]
  000000014001E550: C5 F8 5F 5C 0C 60  vmaxps      xmm3,xmm0,xmmword ptr [rsp+rcx+60h]
  000000014001E556: C5 FA 10 64 C8 A0  vmovss      xmm4,dword ptr [rax+rcx*8-60h]
```


## dual-before

```asm
  000000014001E72E: C5 D2 59 71 84     vmulss      xmm6,xmm5,dword ptr [rcx-7Ch]
  000000014001E733: C5 5A 58 C6        vaddss      xmm8,xmm4,xmm6
  000000014001E737: C5 7A 11 44 24 20  vmovss      dword ptr [rsp+20h],xmm8
  000000014001E73D: C5 D2 59 71 88     vmulss      xmm6,xmm5,dword ptr [rcx-78h]
  000000014001E742: C5 FA 10 9C 24 C0  vmovss      xmm3,dword ptr [rsp+00000000000001C0h]
  000000014001E74B: C5 E2 58 DE        vaddss      xmm3,xmm3,xmm6
  000000014001E74F: C5 FA 11 9C 24 C0  vmovss      dword ptr [rsp+00000000000001C0h],xmm3
  000000014001E758: C5 FA 11 5C 24 24  vmovss      dword ptr [rsp+24h],xmm3
  000000014001E75E: C5 D2 59 71 8C     vmulss      xmm6,xmm5,dword ptr [rcx-74h]
  000000014001E763: C5 FA 10 9C 24 A0  vmovss      xmm3,dword ptr [rsp+00000000000001A0h]
  000000014001E76C: C5 E2 58 DE        vaddss      xmm3,xmm3,xmm6
  000000014001E770: C5 FA 11 9C 24 A0  vmovss      dword ptr [rsp+00000000000001A0h],xmm3
  000000014001E779: C5 FA 11 5C 24 28  vmovss      dword ptr [rsp+28h],xmm3
  000000014001E77F: C5 D2 59 71 90     vmulss      xmm6,xmm5,dword ptr [rcx-70h]
  000000014001E784: C5 FA 10 9C 24 80  vmovss      xmm3,dword ptr [rsp+0000000000000180h]
  000000014001E78D: C5 E2 58 DE        vaddss      xmm3,xmm3,xmm6
  000000014001E791: C5 FA 11 9C 24 80  vmovss      dword ptr [rsp+0000000000000180h],xmm3
  000000014001E79A: C5 FA 11 5C 24 2C  vmovss      dword ptr [rsp+2Ch],xmm3
  000000014001E7A0: C5 D2 59 71 94     vmulss      xmm6,xmm5,dword ptr [rcx-6Ch]
  000000014001E7A5: C5 FA 10 9C 24 60  vmovss      xmm3,dword ptr [rsp+0000000000000160h]
  000000014001E7AE: C5 E2 58 DE        vaddss      xmm3,xmm3,xmm6
  000000014001E7B2: C5 FA 11 9C 24 60  vmovss      dword ptr [rsp+0000000000000160h],xmm3
```


## dual-fast

```asm
  000000014001E5A0: C4 E2 7D 18 A4 0C  vbroadcastss ymm4,dword ptr [rsp+rcx+00000000000004A0h]
  000000014001E5AA: C5 DC 59 A8 A0 F3  vmulps      ymm5,ymm4,ymmword ptr [rax+FFFFFFFFFFFFF3A0h]
  000000014001E5B2: C5 E4 58 DD        vaddps      ymm3,ymm3,ymm5
  000000014001E5B6: C5 DC 59 A8 C0 F3  vmulps      ymm5,ymm4,ymmword ptr [rax+FFFFFFFFFFFFF3C0h]
  000000014001E5BE: C5 EC 58 D5        vaddps      ymm2,ymm2,ymm5
  000000014001E5C2: C5 DC 59 A8 E0 F3  vmulps      ymm5,ymm4,ymmword ptr [rax+FFFFFFFFFFFFF3E0h]
  000000014001E5CA: C5 F4 58 CD        vaddps      ymm1,ymm1,ymm5
  000000014001E5CE: C5 DC 59 A0 00 F4  vmulps      ymm4,ymm4,ymmword ptr [rax+FFFFFFFFFFFFF400h]
  000000014001E5D6: C5 FC 58 C4        vaddps      ymm0,ymm0,ymm4
  000000014001E5DA: C4 E2 7D 18 A4 0C  vbroadcastss ymm4,dword ptr [rsp+rcx+00000000000004A4h]
  000000014001E5E4: C5 DC 59 A8 A0 F7  vmulps      ymm5,ymm4,ymmword ptr [rax+FFFFFFFFFFFFF7A0h]
  000000014001E5EC: C5 E4 58 DD        vaddps      ymm3,ymm3,ymm5
  000000014001E5F0: C5 DC 59 A8 C0 F7  vmulps      ymm5,ymm4,ymmword ptr [rax+FFFFFFFFFFFFF7C0h]
  000000014001E5F8: C5 EC 58 D5        vaddps      ymm2,ymm2,ymm5
  000000014001E5FC: C5 DC 59 A8 E0 F7  vmulps      ymm5,ymm4,ymmword ptr [rax+FFFFFFFFFFFFF7E0h]
  000000014001E604: C5 F4 58 CD        vaddps      ymm1,ymm1,ymm5
  000000014001E608: C5 DC 59 A0 00 F8  vmulps      ymm4,ymm4,ymmword ptr [rax+FFFFFFFFFFFFF800h]
  000000014001E610: C5 FC 58 C4        vaddps      ymm0,ymm0,ymm4
  000000014001E614: C4 E2 7D 18 A4 0C  vbroadcastss ymm4,dword ptr [rsp+rcx+00000000000004A8h]
```


## default-after

```asm
  000000014001E512: C4 E2 7D 18 F6     vbroadcastss ymm6,xmm6
  000000014001E517: C5 CC 59 78 A0     vmulps      ymm7,ymm6,ymmword ptr [rax-60h]
  000000014001E51C: C5 FC 58 C7        vaddps      ymm0,ymm0,ymm7
  000000014001E520: C5 CC 59 78 C0     vmulps      ymm7,ymm6,ymmword ptr [rax-40h]
  000000014001E525: C5 F4 58 CF        vaddps      ymm1,ymm1,ymm7
  000000014001E529: C5 CC 59 78 E0     vmulps      ymm7,ymm6,ymmword ptr [rax-20h]
  000000014001E52E: C5 EC 58 D7        vaddps      ymm2,ymm2,ymm7
  000000014001E532: C5 CC 59 30        vmulps      ymm6,ymm6,ymmword ptr [rax]
  000000014001E536: C5 E4 58 DE        vaddps      ymm3,ymm3,ymm6
  000000014001E53A: 48 05 00 04 00 00  add         rax,400h
  000000014001E540: 48 83 F9 40        cmp         rcx,40h
  000000014001E544: 75 BA              jne         000000014001E500
  000000014001E546: C5 FC 11 84 24 A0  vmovups     ymmword ptr [rsp+00000000000004A0h],ymm0
  000000014001E54F: C5 FC 11 8C 24 C0  vmovups     ymmword ptr [rsp+00000000000004C0h],ymm1
  000000014001E558: C5 FC 11 94 24 E0  vmovups     ymmword ptr [rsp+00000000000004E0h],ymm2
  000000014001E561: C5 FC 11 9C 24 00  vmovups     ymmword ptr [rsp+0000000000000500h],ymm3
  000000014001E56A: C4 A1 7A 10 8C B3  vmovss      xmm1,dword ptr [rbx+r14*4+0000000000205200h]
  000000014001E574: 4A 8D 04 B3        lea         rax,[rbx+r14*4]
  000000014001E578: 48 05 60 4E 20 00  add         rax,204E60h
  000000014001E57E: 31 C9              xor         ecx,ecx
  000000014001E580: C5 F8 57 C0        vxorps      xmm0,xmm0,xmm0
  000000014001E584: C4 E2 79 18 15 13  vbroadcastss xmm2,dword ptr [000000014006D6A0h]
  000000014001E58D: 0F 1F 00           nop         dword ptr [rax]
  000000014001E590: C5 F8 5F 9C 0C A0  vmaxps      xmm3,xmm0,xmmword ptr [rsp+rcx+00000000000004A0h]
```


The final dual build and the timed fixed dual build have identical 441,344-byte .text sections (byte-for-byte); PE timestamps/metadata make whole-file hashes differ.
