<?xml version="1.0" encoding="utf-8"?>
<!-- Hand-authored CityGML 2.0 BuildingParts fixture for the cityparquet W-M4
     round-trip oracle. One bldg:Building "B" with its OWN lod1Solid plus two
     consistsOfBuildingPart children: "B_p1" (a plain lod2Solid) and "B_p2"
     (geometry only in bldg:boundedBy semantic surfaces, no solid). Solids are
     small tetrahedra; coordinates are metric and arbitrary. -->
<CityModel xmlns="http://www.opengis.net/citygml/2.0"
           xmlns:bldg="http://www.opengis.net/citygml/building/2.0"
           xmlns:gen="http://www.opengis.net/citygml/generics/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns:xlink="http://www.w3.org/1999/xlink">
  <cityObjectMember>
    <bldg:Building gml:id="B">
      <bldg:measuredHeight uom="m">10.0</bldg:measuredHeight>
      <bldg:lod1Solid>
        <gml:Solid>
          <gml:exterior>
            <gml:CompositeSurface>
              <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">0 0 0 10 0 0 0 10 0 0 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
              <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">0 0 0 10 0 0 0 0 10 0 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
              <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">0 0 0 0 10 0 0 0 10 0 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
              <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">10 0 0 0 10 0 0 0 10 10 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
            </gml:CompositeSurface>
          </gml:exterior>
        </gml:Solid>
      </bldg:lod1Solid>
      <bldg:consistsOfBuildingPart>
        <bldg:BuildingPart gml:id="B_p1">
          <bldg:lod2Solid>
            <gml:Solid>
              <gml:exterior>
                <gml:CompositeSurface>
                  <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">100 0 0 110 0 0 100 10 0 100 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
                  <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">100 0 0 110 0 0 100 0 10 100 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
                  <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">100 0 0 100 10 0 100 0 10 100 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
                  <gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">110 0 0 100 10 0 100 0 10 110 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
                </gml:CompositeSurface>
              </gml:exterior>
            </gml:Solid>
          </bldg:lod2Solid>
        </bldg:BuildingPart>
      </bldg:consistsOfBuildingPart>
      <bldg:consistsOfBuildingPart>
        <bldg:BuildingPart gml:id="B_p2">
          <bldg:boundedBy>
            <bldg:WallSurface gml:id="B_p2_wall">
              <bldg:lod2MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember><gml:Polygon gml:id="B_p2_wp"><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">200 0 0 210 0 0 200 0 10 200 0 0</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod2MultiSurface>
            </bldg:WallSurface>
          </bldg:boundedBy>
          <bldg:boundedBy>
            <bldg:RoofSurface gml:id="B_p2_roof">
              <bldg:lod2MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember><gml:Polygon gml:id="B_p2_rp"><gml:exterior><gml:LinearRing><gml:posList srsDimension="3">200 0 10 210 0 10 200 10 10 200 0 10</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod2MultiSurface>
            </bldg:RoofSurface>
          </bldg:boundedBy>
        </bldg:BuildingPart>
      </bldg:consistsOfBuildingPart>
    </bldg:Building>
  </cityObjectMember>
</CityModel>
