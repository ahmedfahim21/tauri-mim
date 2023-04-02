import { useState } from "react";
import { invoke } from "@tauri-apps/api/tauri";

function App() {
  const [Msg, setMsg] = useState("");
  const [name, setName] = useState("");

  async function download() {
    // Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
    // setMsg(await invoke("download_fun", { name }));
    try {
      const response = await window.__TAURI__.invoke('my_command',{name})
      if (response.success) {
        console.log('Command succeeded:', response.message)
      } else {
        console.error('Command failed:', response.message)
        if (response.error) {
          console.error('Error message:', response.error)
        }
      }
    } catch (e) {
      console.error('Error:', e)
    }
  }

  return (
    <div className="w-full py-16 px-[30%] text-white ">
      <div className=" mx-auto grid lg:grid-cols-1 items-center justify-between">
            <div className="my-4">
            <h1 className="md:text-4xl sm:text-3xl text-2xl font-bold py-2">MIM - A Torrent Client</h1>
            </div>
            <div className='my-4 flex flex-col sm:flex-row items-center justify-between'>
            
            <form onSubmit={(e) => {
                      e.preventDefault();
                      download();
                     }}>
                <input className=" p-3 flex w-full rounded-md text-black" placeholder="Enter URL" id="url-input"
            onChange={(e) => setName(e.currentTarget.value)}/>
                <button className="bg-[#00df9a] text-black rounded-md font-medium w-[200px] ml-4 my-6 px-6 py-3" type="submit">DOWNLOAD</button>
                </form>
        </div>
      {/* <p>{Msg}</p> */}
    </div>
    </div>
  );
}

export default App;
