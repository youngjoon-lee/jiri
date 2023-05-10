import Link from "next/link";

export default function Footer() {
  return (
    <footer>
      <div className="mx-auto max-w-7xl px-2 sm:px-6 lg:px-8" style={{ position: "absolute", bottom: 20, width: "100%" }}>
        <label className="text-sm font-light leading-tight tracking-tight text-gray-900 flex flex-row">
          <p className="mr-4">© 2023 Youngjoon Lee<br />Powered by <Link href="https://github.com/libp2p/universal-connectivity">Universal Connectivity</Link></p>
        </label>
      </div>
    </footer>
  )
}
