import * as wasm from "jiri-wasm";


const connectToRemoteMultiaddr = document.getElementById("connect-to-remote-multiaddr");

connectToRemoteMultiaddr.addEventListener("click", event => {
  const remoteMultiaddr = document.getElementById("remote-multiaddr").value;
  console.log(`remoteMultiaddr: ${remoteMultiaddr}`);

  const status = document.getElementById("status");
  status.innerHTML = `Connecting to ${remoteMultiaddr}. See console logs...`;

  wasm.start(remoteMultiaddr);
});
