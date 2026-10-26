# Chaves de teste (M29)

Chaves **só para os testes**, sem acesso a nada. Foram geradas uma vez com o `puttygen` 0.80 e
não mudam, para os testes lerem um `.ppk` de verdade (o formato 3, com Argon2id):

```sh
puttygen -t ed25519 -C ana@putty -o ed25519.ppk --new-passphrase /dev/null </dev/null
echo putty-senha > senha.txt
puttygen -t rsa -b 3072 -C ana@putty-rsa -o rsa-senha.ppk --new-passphrase senha.txt
```

- `ed25519.ppk`: Ed25519, comentário `ana@putty`, sem passphrase.
- `rsa-senha.ppk`: RSA 3072, comentário `ana@putty-rsa`, passphrase `putty-senha`.
