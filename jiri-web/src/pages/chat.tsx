import Head from 'next/head'
import Nav from '@/components/nav'
import ChatContainer from '@/components/chat'

export default function Chat() {
  return (
    <>
      <Head>
        <title>JIRI</title>
        <meta name="description" content="Generated by create next app" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <link rel="icon" href="/favicon.ico" />
      </Head>
      <main className="min-h-full">
        <Nav />
        <div className="">
          <main>
            <ChatContainer />
          </main>
        </div>
      </main>
    </>
  )
}
