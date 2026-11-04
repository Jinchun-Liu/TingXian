import {LegacyEntry} from '@/components/interview/LegacyEntry';
export function generateStaticParams(){return ['team-sync-dec-26','product-review','project-ideas','action-items'].map(id=>({id}));}
export default function Page(){return <LegacyEntry panel="history"/>;}
